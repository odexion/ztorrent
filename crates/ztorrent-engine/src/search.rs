//! Online search, run side by side on their own threads, one per source. Every
//! request goes through the same client as a .torrent fetch, so a search leaves
//! by the proxy and the bound interface the user chose, or not at all.
//!
//! Two sources are built in, and both are neutral by nature: the Internet
//! Archive, and a Jackett or Prowlarr server the user runs. Everything else is
//! a [`CustomSource`] the user adds -- a search URL answering in JSON -- read
//! by one generic reader that knows the shape of a search answer, not any site.

use crate::net;
use serde_json::Value;
use ztorrent_core::egress::EgressPolicy;
use ztorrent_core::{CustomSource, SearchOptions, SearchOutcome, SearchResult};

/// An answer is a page of JSON; anything past this is not one.
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;

type Client = reqwest::blocking::Client;
type Run = fn(&Client, &str, &SearchOptions) -> Result<Vec<SearchResult>, String>;

/// By the ids in `SEARCH_SOURCES`.
const PROVIDERS: &[(&str, Run)] = &[("archive", internet_archive), ("indexer", indexer)];

type Job = Box<dyn FnOnce() -> Result<Vec<SearchResult>, String> + Send>;

pub fn search(query: &str, options: &SearchOptions, policy: Option<&EgressPolicy>, version: &str) -> SearchOutcome {
    let query = query.trim().to_string();
    if query.is_empty() {
        return SearchOutcome::default();
    }
    // One client for the lot is fine here: it is built once per search, so a
    // bound interface's address is still read when the search is made.
    let client = match net::client(policy, version) {
        Ok(c) => c,
        Err(err) => return SearchOutcome { results: Vec::new(), failures: vec![err] },
    };
    let mut jobs: Vec<(String, Job)> = Vec::new();
    for &(id, run) in PROVIDERS.iter().filter(|(id, _)| options.sources.iter().any(|s| s == id)) {
        let name = ztorrent_core::SEARCH_SOURCES.iter().find(|(i, _)| *i == id).map_or(id, |(_, n)| n);
        let (client, query, options) = (client.clone(), query.clone(), options.clone());
        jobs.push((name.to_string(), Box::new(move || run(&client, &query, &options))));
    }
    for source in &options.custom {
        let (client, query, source) = (client.clone(), query.clone(), source.clone());
        let category = options.category.clone();
        jobs.push((source.name.clone(), Box::new(move || custom(&client, &source, &query, &category))));
    }
    let workers: Vec<_> = jobs.into_iter().map(|(name, job)| (name, std::thread::spawn(job))).collect();
    let mut out = SearchOutcome::default();
    for (name, worker) in workers {
        match worker.join() {
            Ok(Ok(hits)) => {
                let hits = hits.into_iter().filter(|h| !(options.hide_dead && h.seeders == Some(0)));
                out.results.extend(hits.take(options.max_results));
            }
            Ok(Err(err)) => out.failures.push(format!("{name}: {err}")),
            Err(_) => out.failures.push(format!("{name}: stopped unexpectedly")),
        }
    }
    out
}

/// One search with a source the user is setting up, to show what it finds.
pub fn test_source(source: &CustomSource, query: &str, policy: Option<&EgressPolicy>, version: &str) -> Result<Vec<SearchResult>, String> {
    source.check()?;
    let client = net::client(policy, version)?;
    let query = query.trim();
    let Some(json) = custom_answer(&client, source, query)? else {
        return Err(format!("The lookup address answered, but with no IMDb number for \u{201c}{query}\u{201d}. Try a title it would know."));
    };
    let hits = read_custom(&json, source);
    if hits.is_empty() {
        // Nothing listed at all is the site having nothing for these words; a
        // list whose entries cannot be read is a mapping to fix.
        return Err(if result_list(&json, source).is_empty() {
            format!("The site answered, but had nothing for \u{201c}{query}\u{201d}. Try a test search it would have: a film title for a film site, a show for a TV site.")
        } else {
            "The site answered with results, but ztorrent could not find a magnet link, info hash or .torrent link in them. Set those fields under Advanced, or ask the assistant again.".into()
        });
    }
    Ok(hits)
}

fn get_json(req: reqwest::blocking::RequestBuilder) -> Result<Value, String> {
    use std::io::Read;
    let res = req.send().map_err(|e| net_error(&e))?;
    match res.status().as_u16() {
        200..=299 => {}
        401 | 403 => return Err(format!("access was refused (HTTP {}) -- check the API key", res.status().as_u16())),
        code => return Err(format!("HTTP {code}")),
    }
    let mut body = Vec::new();
    res.take(MAX_BODY_BYTES).read_to_end(&mut body).map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|_| "the answer was not JSON -- the address must be the site's search API, not its web page".to_string())
}

fn net_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "timed out".into()
    } else if err.is_connect() {
        "could not connect".into()
    } else {
        err.to_string()
    }
}

/// Asks for a few more than are shown, since dead hits may be dropped after.
fn wanted(options: &SearchOptions) -> String {
    (options.max_results * 2).min(200).to_string()
}

// ------------------------------------------------------------ Internet Archive

fn internet_archive(client: &Client, query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>, String> {
    let words = lucene_words(query);
    if words.is_empty() {
        return Ok(Vec::new());
    }
    let q = archive_query(&words, &options.category);
    let rows = wanted(options);
    let req = client.get("https://archive.org/advancedsearch.php").query(&[
        ("q", q.as_str()),
        ("fl[]", "identifier"),
        ("fl[]", "title"),
        ("fl[]", "item_size"),
        ("sort[]", "downloads desc"),
        ("rows", &rows),
        ("output", "json"),
    ]);
    Ok(parse_internet_archive(&get_json(req)?))
}

/// Every word must be in the title. A search of the full text, as the archive
/// does by default, finds whatever mentions a word anywhere, and sorted by
/// downloads the most popular of those bury everything relevant.
fn archive_query(words: &str, category: &str) -> String {
    let all = words.split_whitespace().collect::<Vec<_>>().join(" AND ");
    let media = match category {
        "video" => " AND mediatype:movies",
        "audio" => " AND mediatype:(audio OR etree)",
        "books" => " AND mediatype:texts",
        "software" | "games" => " AND mediatype:software",
        _ => "",
    };
    format!("title:({all}){media} AND format:\"Archive BitTorrent\"")
}

/// The user's words with the characters the archive's query language treats as
/// operators taken out, so "c++ (draft)" is a search and not a syntax error.
fn lucene_words(query: &str) -> String {
    let cleaned: String = query.chars().map(|c| if "+-&|!(){}[]^\"~*?:\\/".contains(c) { ' ' } else { c }).collect();
    // Bare AND / OR / NOT would be read as operators too.
    cleaned.split_whitespace().filter(|w| !matches!(*w, "AND" | "OR" | "NOT")).collect::<Vec<_>>().join(" ")
}

fn parse_internet_archive(json: &Value) -> Vec<SearchResult> {
    let docs = json.pointer("/response/docs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    docs.iter()
        .filter_map(|doc| {
            let id = doc.get("identifier")?.as_str()?;
            // Identifiers are file-name safe on the archive; refuse anything that
            // would let a hit point the link somewhere else.
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) || id.starts_with('.') {
                return None;
            }
            let title = doc.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or(id);
            Some(SearchResult {
                source: "Internet Archive".into(),
                title: unescape(title),
                link: format!("https://archive.org/download/{id}/{id}_archive.torrent"),
                size: number(doc.get("item_size")),
                seeders: None,
                leechers: None,
            })
        })
        .collect()
}

// ------------------------------------------------------------ Jackett / Prowlarr

/// Newznab category numbers, which both speak.
fn indexer_categories(category: &str) -> &'static [&'static str] {
    match category {
        "video" => &["2000", "5000"],
        "audio" => &["3000"],
        "books" => &["7000"],
        "software" => &["4000"],
        "games" => &["1000", "4050"],
        _ => &[],
    }
}

fn indexer(client: &Client, query: &str, options: &SearchOptions) -> Result<Vec<SearchResult>, String> {
    let base = options.indexer_url.as_str();
    if base.is_empty() {
        return Err("set its address in the search settings".into());
    }
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err("its address must start with http:// or https://".into());
    }
    if options.indexer_key.is_empty() {
        return Err("set its API key in the search settings".into());
    }
    let cats = indexer_categories(&options.category);
    if options.indexer_kind == "prowlarr" {
        let limit = wanted(options);
        let mut params = vec![("query", query), ("type", "search"), ("limit", limit.as_str())];
        params.extend(cats.iter().map(|c| ("categories", *c)));
        let req = client.get(format!("{base}/api/v1/search")).header("X-Api-Key", &options.indexer_key).query(&params);
        Ok(parse_prowlarr(&get_json(req)?))
    } else {
        let mut params = vec![("apikey", options.indexer_key.as_str()), ("Query", query)];
        params.extend(cats.iter().map(|c| ("Category[]", *c)));
        let req = client.get(format!("{base}/api/v2.0/indexers/all/results")).query(&params);
        Ok(parse_jackett(&get_json(req)?))
    }
}

/// A magnet if there is one, else a download link: an indexer's link to a
/// .torrent sometimes answers with a redirect to a magnet, which a magnet avoids.
fn pick_link(magnet: Option<&Value>, download: Option<&Value>) -> Option<String> {
    let magnet = magnet.and_then(Value::as_str).filter(|m| m.starts_with("magnet:"));
    let download = download.and_then(Value::as_str).filter(|d| d.starts_with("http://") || d.starts_with("https://") || d.starts_with("magnet:"));
    magnet.or(download).map(String::from)
}

fn parse_jackett(json: &Value) -> Vec<SearchResult> {
    let items = json.get("Results").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let mut hits: Vec<SearchResult> = items
        .iter()
        .filter_map(|item| {
            let seeders = number(item.get("Seeders"));
            Some(SearchResult {
                source: item.get("Tracker").and_then(Value::as_str).unwrap_or("Jackett").to_string(),
                title: item.get("Title")?.as_str()?.to_string(),
                link: pick_link(item.get("MagnetUri"), item.get("Link"))?,
                size: number(item.get("Size")),
                seeders: Some(seeders as u32),
                // Jackett counts every peer, seeders included.
                leechers: Some(number(item.get("Peers")).saturating_sub(seeders) as u32),
            })
        })
        .collect();
    hits.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    hits
}

fn parse_prowlarr(json: &Value) -> Vec<SearchResult> {
    let mut hits: Vec<SearchResult> = json
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        // Prowlarr also answers with Usenet releases, which are no use here.
        .filter(|item| item.get("protocol").and_then(Value::as_str).is_none_or(|p| p == "torrent"))
        .filter_map(|item| {
            Some(SearchResult {
                source: item.get("indexer").and_then(Value::as_str).unwrap_or("Prowlarr").to_string(),
                title: item.get("title")?.as_str()?.to_string(),
                link: pick_link(item.get("magnetUrl"), item.get("downloadUrl"))?,
                size: number(item.get("size")),
                seeders: Some(number(item.get("seeders")) as u32),
                leechers: Some(number(item.get("leechers")) as u32),
            })
        })
        .collect();
    hits.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    hits
}

// ------------------------------------------------------------- custom sources

/// The names search APIs give each field, most specific first. Compared
/// without case, `_` or `-`, so `magnet_url`, `magnetUrl` and `MagnetURL` are one.
const TITLE_KEYS: &[&str] = &["title", "name", "titlelong", "filename", "release", "torrentname"];
const HASH_KEYS: &[&str] = &["infohash", "hash", "btih"];
const MAGNET_KEYS: &[&str] = &["magnet", "magneturl", "magneturi", "magnetlink"];
const LINK_KEYS: &[&str] = &["torrenturl", "downloadurl", "torrentlink", "downloadlink", "torrent", "download"];
/// Names that may as well hold the result's web page, so trusted only when the
/// link itself looks like a torrent.
const LOOSE_LINK_KEYS: &[&str] = &["link", "url"];
const SIZE_KEYS: &[&str] = &["size", "sizebytes", "bytes", "length", "totalsize", "itemsize", "filesize"];
const SEEDER_KEYS: &[&str] = &["seeders", "seeds", "seed", "seeder"];
const LEECHER_KEYS: &[&str] = &["leechers", "leeches", "leecher", "peers"];
/// What tells apart the versions of one result, e.g. "1080p web".
const VERSION_KEYS: &[&str] = &["quality", "resolution", "type", "videocodec", "codec", "format"];

fn custom(client: &Client, source: &CustomSource, query: &str, category: &str) -> Result<Vec<SearchResult>, String> {
    // A source has no notion of categories: it is asked the words alone, and
    // searched under any category, since nothing is known of what it holds.
    let _ = category;
    let Some(json) = custom_answer(client, source, query)? else { return Ok(Vec::new()) };
    let mut hits = read_custom(&json, source);
    hits.sort_by(|a, b| b.seeders.cmp(&a.seeders));
    Ok(hits)
}

/// The IMDb id for the search words, from the source's lookup address: the
/// first one in its answer. The words themselves, when they are an id. None
/// when the lookup knows no title by them.
fn imdb_id(client: &Client, source: &CustomSource, query: &str) -> Result<Option<String>, String> {
    if let Some(id) = imdb_in(query).filter(|id| id == query) {
        return Ok(Some(id));
    }
    let url = source.lookup.trim().replace(CustomSource::QUERY, &percent_encode(query));
    match get_json(client.get(url)) {
        Ok(json) => Ok(first_imdb(&json)),
        // How a lookup says it has nothing by that name.
        Err(err) if err == "HTTP 404" => Ok(None),
        Err(err) => Err(format!("lookup at {}: {err}", host(&source.lookup))),
    }
}

/// The first IMDb id in an answer, in the order it lists its titles.
fn first_imdb(json: &Value) -> Option<String> {
    match json {
        Value::String(s) => imdb_in(s),
        Value::Array(list) => list.iter().find_map(first_imdb),
        Value::Object(map) => map.values().find_map(first_imdb),
        _ => None,
    }
}

/// An IMDb id, `tt` and seven or more digits, alone or inside a link.
fn imdb_in(text: &str) -> Option<String> {
    text.match_indices("tt").find_map(|(at, _)| {
        let before = text[..at].chars().next_back();
        let digits: String = text[at + 2..].chars().take_while(char::is_ascii_digit).collect();
        (!before.is_some_and(|c| c.is_ascii_alphanumeric()) && (7..=10).contains(&digits.len())).then(|| format!("tt{digits}"))
    })
}

/// The first answer from the source's address or one of its backups. None
/// when its lookup found no IMDb id to ask it for.
fn custom_answer(client: &Client, source: &CustomSource, query: &str) -> Result<Option<Value>, String> {
    let imdb = if source.lookup.trim().is_empty() {
        String::new()
    } else {
        match imdb_id(client, source, query)? {
            Some(id) => id,
            None => return Ok(None),
        }
    };
    let addresses: Vec<&String> = std::iter::once(&source.url).chain(&source.mirrors).collect();
    let mut failures = Vec::new();
    let mut answer = None;
    for address in &addresses {
        let url = address
            .trim()
            .replace(CustomSource::QUERY, &percent_encode(query))
            .replace(CustomSource::IMDB_NUMBER, imdb.trim_start_matches("tt"))
            .replace(CustomSource::IMDB, &imdb);
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return Err("its address must start with https:// or http://".into());
        }
        match get_json(client.get(url)) {
            Ok(json) => {
                answer = Some(json);
                break;
            }
            Err(err) => failures.push(format!("{}: {err}", host(address))),
        }
    }
    let Some(json) = answer else {
        let moved = failures.iter().any(|f| f.ends_with("could not connect") || f.ends_with("timed out") || f.contains("HTTP 404") || f.contains("HTTP 5"));
        let hint = if moved { ". The site may have moved to a new address: ask for the current one, or add it as a backup address (\"mirrors\")" } else { "" };
        return Err(format!("{}{hint}", failures.join("; ")));
    };
    Ok(Some(json))
}

/// The host part of an address, to name it in a message.
fn host(url: &str) -> &str {
    url.split("://").nth(1).and_then(|r| r.split(['/', '?']).next()).unwrap_or(url)
}

/// Reads the results out of any search answer, by the source's mapping where it
/// sets one and by the usual field names where it does not.
fn read_custom(json: &Value, source: &CustomSource) -> Vec<SearchResult> {
    let list = result_list(json, source);
    let mut hits = Vec::new();
    for item in list.iter().filter(|i| i.is_object()) {
        let versions = if source.items.trim().is_empty() {
            find_versions(item)
        } else {
            path(item, &source.items).and_then(Value::as_array).map(|a| a.as_slice()).or_else(|| find_versions(item))
        };
        match versions {
            Some(versions) => hits.extend(versions.iter().filter_map(|v| read_hit(v, Some(item), source))),
            None => hits.extend(read_hit(item, None, source)),
        }
    }
    hits
}

/// The list of results in an answer, where the mapping says or where it looks to be.
fn result_list<'a>(json: &'a Value, source: &CustomSource) -> &'a [Value] {
    if source.results.trim().is_empty() {
        find_results(json)
    } else {
        // A path that is not to a list (an assistant's "0", say) is ignored.
        path(json, &source.results).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_else(|| find_results(json))
    }
}

/// One result: a field is looked for on the torrent, then on the result it is a
/// version of.
fn read_hit(entry: &Value, parent: Option<&Value>, source: &CustomSource) -> Option<SearchResult> {
    // A path written from the result down to its versions ("torrents.hash" when
    // the versions are "torrents") is read from the version too.
    let items = normalise_path(&source.items);
    let within = |set: &str| -> String {
        let set = normalise_path(set);
        match set.strip_prefix(&format!("{items}.")) {
            Some(rest) if parent.is_some() && !items.is_empty() => rest.to_string(),
            _ => set,
        }
    };
    let field = |set: &str, keys: &[&str]| {
        pick(entry, &within(set), keys).or_else(|| parent.and_then(|p| pick(p, set, keys))).filter(|v| !v.is_null())
    };
    let text = |v: &Value| match v {
        Value::String(s) => Some(s.trim().to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    };
    let mut title = unescape(&field(&source.title, TITLE_KEYS).and_then(text).filter(|t| !t.is_empty())?);
    // A version of a result has no name of its own: say which version it is.
    if parent.is_some() {
        if pick(entry, &within(&source.title), TITLE_KEYS).is_none() {
            let tags: Vec<String> = VERSION_KEYS.iter().filter_map(|k| lookup(entry, &[k]).and_then(text)).filter(|t| !t.is_empty()).collect();
            if !tags.is_empty() {
                title = format!("{title} [{}]", tags.join(" "));
            }
        }
    }
    let magnet = field(&source.magnet, MAGNET_KEYS).and_then(Value::as_str).filter(|m| m.starts_with("magnet:")).map(String::from);
    let from_hash = || {
        let hash = valid_hash(field(&source.hash, HASH_KEYS)?.as_str()?)?;
        let trackers: String = source.trackers.iter().filter(|t| !t.trim().is_empty()).map(|t| format!("&tr={}", percent_encode(t.trim()))).collect();
        Some(format!("magnet:?xt=urn:btih:{hash}&dn={}{trackers}", percent_encode(&title)))
    };
    let from_link = || {
        let web = |l: &str| l.starts_with("https://") || l.starts_with("http://");
        let named = field(&source.link, LINK_KEYS).and_then(Value::as_str).filter(|l| l.starts_with("magnet:") || web(l));
        // A mapping that names the field is trusted; the loose names are a guess.
        let loose = || {
            if !source.link.trim().is_empty() {
                return None;
            }
            field("", LOOSE_LINK_KEYS).and_then(Value::as_str).filter(|l| l.starts_with("magnet:") || (web(l) && looks_like_torrent(l)))
        };
        named.or_else(loose).map(String::from)
    };
    let link = magnet.or_else(from_hash).or_else(from_link)?;
    let count = |set: &str, keys: &[&str]| field(set, keys).map(|v| number(Some(v)) as u32);
    Some(SearchResult {
        source: source.name.clone(),
        size: field(&source.size, SIZE_KEYS).map(size).unwrap_or(0),
        seeders: count(&source.seeders, SEEDER_KEYS),
        leechers: count(&source.leechers, LEECHER_KEYS),
        title,
        link,
    })
}

/// The field the mapping names, or else one by its usual names.
/// A mapping is a hint: one that leads nowhere -- an assistant's guess, a
/// field the site has since renamed -- falls back to the usual names.
fn pick<'a>(v: &'a Value, set: &str, keys: &[&str]) -> Option<&'a Value> {
    if set.trim().is_empty() { lookup(v, keys) } else { path(v, set).or_else(|| lookup(v, keys)) }
}

/// Whether a link says it is a torrent rather than the result's web page.
fn looks_like_torrent(link: &str) -> bool {
    let l = link.to_ascii_lowercase();
    l.contains(".torrent") || l.contains("/download")
}

/// The list in the answer that looks most like search results: the one whose
/// entries have the most of the usual fields. The answer itself, if a list.
fn find_results(json: &Value) -> &[Value] {
    fn score(list: &[Value]) -> usize {
        let Some(first) = list.iter().find(|v| v.is_object()) else { return 0 };
        let has = |keys: &[&str]| lookup(first, keys).is_some() as usize;
        let has_title = has(TITLE_KEYS);
        if has_title == 0 {
            return 0;
        }
        let linked = has(HASH_KEYS) | has(MAGNET_KEYS) | has(LINK_KEYS) | has(LOOSE_LINK_KEYS) | find_versions(first).is_some() as usize;
        has_title + 2 * linked + has(SIZE_KEYS) + has(SEEDER_KEYS)
    }
    fn walk<'a>(v: &'a Value, depth: usize, best: &mut (usize, &'a [Value])) {
        match v {
            Value::Array(list) => {
                let s = score(list);
                if s > best.0 || (s == best.0 && s > 0 && list.len() > best.1.len()) {
                    *best = (s, list);
                }
            }
            Value::Object(map) if depth < 4 => map.values().for_each(|c| walk(c, depth + 1, best)),
            _ => {}
        }
    }
    let mut best: (usize, &[Value]) = (0, &[]);
    walk(json, 0, &mut best);
    best.1
}

/// The versions of a result that holds no torrent itself but a list of them,
/// as a film offered in several qualities does.
fn find_versions(item: &Value) -> Option<&[Value]> {
    let linked = |v: &Value| lookup(v, HASH_KEYS).is_some() || lookup(v, MAGNET_KEYS).is_some();
    if linked(item) {
        return None;
    }
    item.as_object()?.values().filter_map(Value::as_array).find(|list| list.first().is_some_and(|v| v.is_object() && linked(v))).map(|l| l.as_slice())
}

/// A field by any of its usual names, ignoring case, `_` and `-`.
fn lookup<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let map = v.as_object()?;
    let norm = |k: &str| k.chars().filter(|c| *c != '_' && *c != '-').collect::<String>().to_ascii_lowercase();
    keys.iter().find_map(|want| map.iter().find(|(k, val)| norm(k) == *want && !val.is_null()).map(|(_, val)| val))
}

/// A dotted path into the answer: `data.movies`, `results.0.torrents`.
fn path<'a>(v: &'a Value, dotted: &str) -> Option<&'a Value> {
    normalise_path(dotted).split('.').filter(|p| !p.is_empty()).try_fold(v, |v, part| match v {
        Value::Array(list) => list.get(part.parse::<usize>().ok()?),
        _ => v.get(part),
    })
}

/// A path as people and assistants write it -- `$.data.movies[*]`,
/// `data.movies[0].torrents`, `/data/movies` -- in the dotted form read here.
fn normalise_path(p: &str) -> String {
    let p = p.trim().trim_start_matches('$').replace("[*]", "").replace("[]", "").replace('[', ".").replace(']', "").replace('/', ".");
    p.split('.').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(".")
}

/// A size as bytes, or as text such as "1.4 GB" or "700 MiB".
fn size(v: &Value) -> u64 {
    let Value::String(s) = v else { return number(Some(v)) };
    let s = s.trim().replace(',', "");
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let Ok(n) = num.parse::<f64>() else { return 0 };
    let power = match unit.trim().to_ascii_lowercase().trim_end_matches('b').trim_end_matches('i') {
        "" => 0,
        "k" => 1,
        "m" => 2,
        "g" => 3,
        "t" => 4,
        _ => return 0,
    };
    (n * 1024f64.powi(power)) as u64
}

// ---------------------------------------------------------------------- shared

/// A 40-character hex info hash, lowercased; None for anything else, including
/// the all-zero placeholder some sites answer an empty search with.
fn valid_hash(hash: &str) -> Option<String> {
    let hash = hash.trim();
    (hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit()) && !hash.chars().all(|c| c == '0')).then(|| hash.to_ascii_lowercase())
}

/// Titles arrive with some characters still escaped for HTML: &amp; and &#124;.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let c = match &rest[1..end] {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                e => e
                    .strip_prefix("#x")
                    .or_else(|| e.strip_prefix("#X"))
                    .map(|h| u32::from_str_radix(h, 16))
                    .or_else(|| e.strip_prefix('#').map(|d| d.parse()))
                    .and_then(Result::ok)
                    .and_then(char::from_u32),
            }?;
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Sites write numbers as numbers or as strings, by field and by day.
fn number(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)).unwrap_or(0),
        Some(Value::String(s)) => s.trim().replace(',', "").parse().unwrap_or(0),
        _ => 0,
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HASH: &str = "2C6B6858D61DA9543D4231A71DB4B1C9264B0685";

    fn src() -> CustomSource {
        CustomSource { name: "Mine".into(), url: "https://example.org/api?q={query}".into(), ..Default::default() }
    }

    #[test]
    fn a_lookup_answer_gives_its_first_imdb_id() {
        let one = json!({"name": "Show", "externals": {"imdb": "tt0944947", "tvrage": 24493}});
        assert_eq!(first_imdb(&one).as_deref(), Some("tt0944947"));
        let many = json!([{"show": {"externals": {"imdb": null}}}, {"show": {"url": "https://www.imdb.com/title/tt1234567/"}}]);
        assert_eq!(first_imdb(&many).as_deref(), Some("tt1234567"));
        assert_eq!(first_imdb(&json!({"title": "Seattle", "id": "att1234567", "n": "tt123"})), None);
    }

    #[test]
    fn a_source_by_imdb_needs_a_lookup() {
        let mut s = CustomSource { name: "Mine".into(), url: "https://example.org/api?imdb_id={imdb_number}".into(), ..Default::default() };
        assert!(s.check().is_err());
        s.lookup = "https://example.org/lookup?q={query}".into();
        assert!(s.check().is_ok());
        s.url = "https://example.org/api?q={query}".into();
        assert!(s.check().is_err());
    }

    #[test]
    fn every_provider_has_a_name_and_every_name_a_provider() {
        let ids: Vec<&str> = PROVIDERS.iter().map(|(id, _)| *id).collect();
        let named: Vec<&str> = ztorrent_core::SEARCH_SOURCES.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, named);
    }

    #[test]
    fn archive_hits_link_to_their_torrent() {
        let hits = parse_internet_archive(&json!({"response": {"docs": [
            {"identifier": "tiny-11", "title": "Tiny11", "item_size": 1234},
            {"identifier": "no-title"},
            {"identifier": "../evil", "title": "x"},
        ]}}));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].link, "https://archive.org/download/tiny-11/tiny-11_archive.torrent");
        assert_eq!((hits[0].title.as_str(), hits[0].size), ("Tiny11", 1234));
        assert_eq!(hits[1].title, "no-title");
        assert!(parse_internet_archive(&json!({"error": "bad query"})).is_empty());
    }

    #[test]
    fn archive_query_needs_every_word_in_the_title() {
        assert_eq!(archive_query("night living dead", "all"), "title:(night AND living AND dead) AND format:\"Archive BitTorrent\"");
        assert!(archive_query("x", "books").contains(" AND mediatype:texts "));
        assert_eq!(lucene_words(" c++  (draft) \"x\" "), "c draft x");
        assert_eq!(lucene_words("cats AND dogs"), "cats dogs");
    }

    #[test]
    fn jackett_prefers_magnets_and_counts_leechers_apart() {
        let hits = parse_jackett(&json!({"Results": [
            {"Title": "A", "Tracker": "Site", "Size": 10, "Seeders": 4, "Peers": 9, "MagnetUri": "magnet:?xt=urn:btih:aa", "Link": "http://j/dl/1"},
            {"Title": "B", "Size": 5, "Seeders": 8, "Peers": 8, "MagnetUri": null, "Link": "http://j/dl/2"},
            {"Title": "No link", "Seeders": 1},
        ]}));
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].title.as_str(), hits[0].link.as_str(), hits[0].source.as_str()), ("B", "http://j/dl/2", "Jackett"));
        assert_eq!((hits[1].link.as_str(), hits[1].leechers), ("magnet:?xt=urn:btih:aa", Some(5)));
    }

    #[test]
    fn prowlarr_skips_usenet() {
        let hits = parse_prowlarr(&json!([
            {"title": "T", "indexer": "Site", "protocol": "torrent", "size": 3, "seeders": 2, "leechers": 1, "downloadUrl": "http://p/1"},
            {"title": "U", "protocol": "usenet", "downloadUrl": "http://p/2"},
        ]));
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].source.as_str(), hits[0].link.as_str()), ("Site", "http://p/1"));
    }

    #[test]
    fn an_indexer_without_settings_says_what_is_missing() {
        let client = Client::new();
        let mut o = SearchOptions { indexer_kind: "jackett".into(), ..Default::default() };
        assert!(indexer(&client, "x", &o).unwrap_err().contains("address"));
        o.indexer_url = "ftp://x".into();
        assert!(indexer(&client, "x", &o).unwrap_err().contains("http"));
        o.indexer_url = "http://127.0.0.1:9117".into();
        assert!(indexer(&client, "x", &o).unwrap_err().contains("API key"));
    }

    #[test]
    fn a_flat_list_of_hashes_is_read_without_a_mapping() {
        let hits = read_custom(
            &json!([
                {"id": "1", "name": "Ubuntu 22.04 LTS", "info_hash": HASH, "seeders": "31", "leechers": "1", "size": "3654957056"},
                {"id": "0", "name": "No results returned", "info_hash": "0000000000000000000000000000000000000000", "seeders": "0"},
            ]),
            &src(),
        );
        assert_eq!(hits.len(), 1, "the all-zero placeholder is not a result");
        let h = &hits[0];
        assert_eq!((h.title.as_str(), h.size, h.seeders, h.leechers, h.source.as_str()), ("Ubuntu 22.04 LTS", 3654957056, Some(31), Some(1), "Mine"));
        assert_eq!(h.link, format!("magnet:?xt=urn:btih:{}&dn=Ubuntu%2022.04%20LTS", HASH.to_ascii_lowercase()));
    }

    #[test]
    fn nested_versions_are_found_and_named() {
        let answer = json!({"status": "ok", "data": {"count": 1, "movies": [{"title_long": "Film (1999)", "torrents": [
            {"hash": HASH, "quality": "720p", "type": "web", "seeds": 10, "peers": 3, "size_bytes": 500},
            {"hash": HASH, "quality": "1080p", "seeds": 20, "peers": 1, "size_bytes": 900},
        ]}]}});
        let mut source = src();
        source.trackers = vec!["udp://tracker.example:1337/announce".into()];
        let hits = read_custom(&answer, &source);
        assert_eq!(hits.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(), ["Film (1999) [720p web]", "Film (1999) [1080p]"]);
        assert_eq!((hits[1].seeders, hits[1].size), (Some(20), 900));
        assert!(hits[0].link.ends_with("&tr=udp%3A%2F%2Ftracker.example%3A1337%2Fannounce"));
    }

    #[test]
    fn magnets_and_torrent_links_are_used_as_they_are() {
        let hits = read_custom(
            &json!({"results": [
                {"Title": "A", "MagnetURL": "magnet:?xt=urn:btih:abc", "Size": "1.5 GB"},
                {"title": "B", "download_url": "https://example.org/get/2"},
                {"title": "C", "url": "https://example.org/page/3"},
                {"title": "D", "url": "https://example.org/files/d.torrent", "seeds": 2},
            ]}),
            &src(),
        );
        assert_eq!(hits.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(), ["A", "B", "D"], "a page link is not a torrent");
        assert_eq!((hits[0].link.as_str(), hits[0].size, hits[0].seeders), ("magnet:?xt=urn:btih:abc", 1610612736, None));
    }

    #[test]
    fn a_mapping_overrides_the_guesses() {
        let answer = json!({"page": {"rows": [{"label": "X", "id": HASH, "name": "not this"}]}});
        let source = CustomSource { results: "page.rows".into(), title: "label".into(), hash: "id".into(), ..src() };
        let hits = read_custom(&answer, &source);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "X");
    }

    #[test]
    fn paths_written_from_the_result_or_as_jsonpath_still_read() {
        let answer = json!({"data": {"movies": [{"title": "Film", "torrents": [{"hash": HASH, "seeds": 7, "quality": "720p"}]}]}});
        let source = CustomSource {
            results: "$.data.movies[*]".into(),
            items: "torrents".into(),
            title: "title".into(),
            hash: "torrents.hash".into(),
            seeders: "torrents[].seeds".into(),
            ..src()
        };
        let hits = read_custom(&answer, &source);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].seeders, hits[0].title.as_str()), (Some(7), "Film [720p]"));
        assert!(hits[0].link.starts_with("magnet:?xt=urn:btih:"));
        assert_eq!(normalise_path("/data/movies[0].x"), "data.movies.0.x");
    }

    #[test]
    fn an_unreachable_address_falls_back_to_a_mirror_and_says_it_may_have_moved() {
        let client = Client::builder().timeout(std::time::Duration::from_secs(2)).build().unwrap();
        // Nothing listens on port 9 of this machine: the connection is refused at once.
        let dead = CustomSource { url: "http://127.0.0.1:9/api?q={query}".into(), ..src() };
        let err = custom(&client, &dead, "x", "").unwrap_err();
        assert!(err.contains("127.0.0.1:9") && err.contains("moved"), "{err}");
        let both = CustomSource { mirrors: vec!["http://127.0.0.1:9/other?q={query}".into()], ..dead };
        assert_eq!(custom(&client, &both, "x", "").unwrap_err().matches("127.0.0.1:9").count(), 2, "every address is tried");
    }

    #[test]
    fn a_mapping_that_leads_nowhere_falls_back_to_finding_fields() {
        let answer = json!([{"name": "Ubuntu", "info_hash": HASH, "seeders": "31", "size": "100"}]);
        // As an assistant wrote it: "0" is the first result, not the list.
        let source = CustomSource { results: "0".into(), title: "name".into(), hash: " info_hash".into(), seeders: "seeds".into(), ..src() };
        let hits = read_custom(&answer, &source);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].title.as_str(), hits[0].seeders, hits[0].size), ("Ubuntu", Some(31), 100));
    }

    #[test]
    fn the_list_with_results_beats_other_lists() {
        let answer = json!({"tags": [{"name": "a"}, {"name": "b"}, {"name": "c"}], "hits": [{"name": "Real", "info_hash": HASH}]});
        assert_eq!(read_custom(&answer, &src()).len(), 1);
        assert!(read_custom(&json!({"message": "nothing"}), &src()).is_empty());
    }

    #[test]
    fn sizes_read_as_bytes_or_text() {
        assert_eq!(size(&json!(42)), 42);
        assert_eq!(size(&json!("1,024")), 1024);
        assert_eq!(size(&json!("700 MiB")), 700 * 1024 * 1024);
        assert_eq!(size(&json!("2KB")), 2048);
        assert_eq!(size(&json!("big")), 0);
    }

    #[test]
    fn paths_walk_objects_and_lists() {
        let v = json!({"a": {"b": [{"c": 1}]}});
        assert_eq!(path(&v, "a.b.0.c"), Some(&json!(1)));
        assert_eq!(path(&v, "a.x"), None);
    }

    #[test]
    fn html_escapes_in_titles_are_decoded() {
        assert_eq!(unescape("Vols 1&amp;2 &#124; x &#x41; &bogus; a & b"), "Vols 1&2 | x A &bogus; a & b");
    }

    #[test]
    fn the_query_goes_where_the_placeholder_is_encoded() {
        let client = Client::new();
        let bad = CustomSource { url: "ftp://example.org/?q={query}".into(), ..src() };
        assert!(custom(&client, &bad, "x", "").unwrap_err().contains("https://"));
        assert_eq!("https://e.org/?q={query}".replace(CustomSource::QUERY, &percent_encode("a b&c")), "https://e.org/?q=a%20b%26c");
    }

    /// Goes to the real site: `cargo test -p ztorrent-engine live_search -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_search() {
        for (query, category) in [("night of the living dead", "all"), ("sherlock holmes", "books")] {
            let options = SearchOptions { sources: vec!["archive".into()], category: category.into(), max_results: 5, hide_dead: true, ..Default::default() };
            let out = search(query, &options, None, "test");
            println!("== {query} [{category}] failures={:?}", out.failures);
            for r in &out.results {
                println!("  {:<16} {:>5?}  {}", r.source, r.seeders, r.title);
            }
            assert!(out.failures.is_empty());
        }
        // A source of your own, by its address: ZTORRENT_TEST_SOURCE='https://…?q={query}'
        // Or a whole definition as pasted: ZTORRENT_TEST_DEFINITION='{"name": …}'
        let pasted = std::env::var("ZTORRENT_TEST_DEFINITION").ok().map(|d| CustomSource::from_shared(&d).unwrap());
        let by_url = std::env::var("ZTORRENT_TEST_SOURCE").ok().map(|url| CustomSource { name: "test".into(), url, ..Default::default() });
        if let Some(source) = pasted.or(by_url) {
            let query = std::env::var("ZTORRENT_TEST_QUERY").unwrap_or_else(|_| "ubuntu".into());
            let hits = test_source(&source, &query, None, "test").unwrap();
            for r in hits.iter().take(5) {
                println!("  {:>5?} {:>12} {}  {}", r.seeders, r.size, r.title, &r.link[..r.link.len().min(60)]);
            }
        }
    }
}
