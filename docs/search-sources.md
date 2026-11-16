# Search sources

**Search Online** (the globe button, or ⇧⌘F / Ctrl+Shift+F) searches the sources
you have turned on, all at once, and lists what they find. Click **Add** on a
result to download it the usual way.

ztorrent comes with no third-party torrent sites. Out of the box it searches:

- **Internet Archive**: a non-profit library of free, legal films, music, books
  and software.
- **Your own search server**: off, and only for people who run Jackett or
  Prowlarr: free servers you run yourself, which search the sites you choose in
  them.

Every other source is one you add. **You choose the sources you add, and you are
responsible for them and for what you download.** See [LEGAL.md](../LEGAL.md).

## Add a source in three steps

Open **Search Online**, click the **gear** next to the Search button, then
**Add a source…**.

### 1. Find the site's search API

You need an address that answers a search with **JSON** (data), not with a web
page. Many sites describe theirs on a page called "API", "Developers" or "Docs".

To check an address, open it in your browser. If you see a page of text full of
`{`, `"`, and `:` characters instead of a normal web page, it is JSON.

### 2. Put `{query}` where the search words go

Search the API for something, copy the address, and replace your search words
with `{query}`:

```
https://example.org/api/search?q=ubuntu
                    becomes
https://example.org/api/search?q={query}
```

Give it a name, paste the address into **Address**, and you are almost done.
(An address is one line starting with `https://`. If what you have is JSON in
`{ … }`, it is a definition: see the next section.)

### 3. Click Test, then Save

ztorrent runs the **Test search** and shows the first few results it found. It
works out by itself where the results are and which fields hold the title, size,
seeders, and the magnet link, info hash or `.torrent` link.

If the preview looks right, click **Save this source** next to it. Adding a source
under a name you already use replaces the old one, which is how you fix a source
that stopped working. The new source is searched along with
the others from then on. Turn it off or on with its checkbox, or **Edit** or
**Remove** it.

## Can't find the API? Ask an AI assistant

Finding a site's API, and the right address for it, is the hard part. The
**Add a source** form can write the question for you:

1. Click **Copy question for an AI**.
2. Paste the question into an AI assistant. It ends with "for the following
   website ", so type the site's name or address straight after it, and send.
3. The assistant is told to reply with nothing but a short piece of JSON, in
   exactly the format ztorrent reads. Paste its reply into the box under the
   button. ztorrent fills in **Name** and **Address** for you, and shows any
   `note` the assistant added, such as where to get an API key. If the assistant
   found no API, it replies with an `error`, and ztorrent shows that instead.
4. Click **Test**. Assistants can be wrong or make up an address, so check that the
   preview shows real results before you click **Save this source**.

## When Test finds nothing

- **"the answer was not JSON"**: the address is the site's web page, not its API.
  Go back to step 1.
- **"access was refused"**: the API needs a key. Many APIs accept it in the address
  (`…?q={query}&apikey=YOUR_KEY`); the site's API page says how.
- **"could not connect … the site may have moved"**: the address is out of date.
  Ask the assistant for the current one, or add it under **Backup addresses**.
- **"had nothing for …"**: the site works but has nothing for the test word. Try a
  word it would have, such as a film title for a film site.
- **"could not find a magnet link, info hash or .torrent link"**: the site answers
  with results, but in an unusual shape. Open **Advanced** and name the fields
  yourself (below), or ask the assistant again.

## Advanced: naming the fields yourself

Leave a field empty and ztorrent finds it. Fill one in to tell ztorrent exactly
where to look, as a dotted path into the JSON:

| Field | What it is | Example |
|---|---|---|
| Results list | Where the list of results is | `data.results` |
| Versions list | A list inside each result, when one result comes in several versions (such as one film in 720p and 1080p) | `torrents` |
| Title | The result's name | `name` |
| Info hash | The 40-character torrent hash | `info_hash` |
| Magnet link | A `magnet:` link | `magnet` |
| Torrent file link | A link to a `.torrent` file | `download_url` |
| Size | Size in bytes, or text such as `1.4 GB` | `size` |
| Seeders / Leechers | How many share it | `seeders` |
| Backup addresses | Other addresses of the same search, each with `{query}`. Sites like this move domains; ztorrent tries these in turn when the main one cannot be reached. | `https://mirror.example.org/api?q={query}` |
| Trackers for hashes | Trackers to add to magnet links ztorrent builds from an info hash. Optional: without any, ztorrent finds peers through the DHT. | `udp://tracker.example.org:1337/announce` |

| Lookup address | For a site that searches by IMDb number only: see below. | `https://example.org/lookup?q={query}` |

Numbers in a path pick from a list, so `results.0.files` is the `files` of the
first result.

For example, with this answer:

```json
{
  "data": {
    "items": [
      { "label": "Some Film (1999)", "versions": [
          { "quality": "1080p", "btih": "…40 hex characters…", "bytes": 1990000000, "seeds": 120 }
      ] }
    ]
  }
}
```

ztorrent already finds everything on its own. If it did not, you would set
*Results list* to `data.items`, *Versions list* to `versions`, *Title* to `label`,
and *Info hash* to `btih`.

## Sites that search by IMDb number, not by words

Some APIs cannot search by title. They want a film's or show's IMDb number, the
`tt0944947` in its address on imdb.com. To search such a site by title, give the
source two addresses:

- **Address**: the site's API, with `{imdb}` where the number goes
  (`tt0944947`), or `{imdb_number}` if the API wants it without the `tt`
  (`0944947`):
  `https://example.org/api/torrents?imdb_id={imdb_number}`
- **Lookup address** (under **Advanced**): any JSON API that answers a title with
  its IMDb number, with `{query}` where the words go:
  `https://example.org/lookup?q={query}`

When you search, ztorrent asks the lookup address for your words, takes the first
IMDb number in its answer, and asks the site for that. Typing an IMDb number
(`tt0944947`) as your search skips the lookup. The lookup address is a source
like any other: you choose it, and it sees your search words and IP address.

## Sharing a source

**Copy** next to a source puts its definition on the clipboard, as a short piece of
JSON:

```json
{
  "name": "Example",
  "url": "https://example.org/api/search?q={query}"
}
```

To add a source someone shared with you, paste the whole definition into the
answer box under **Copy question for an AI**: it fills in every field. Then click
**Test**.

## Your own search server (Jackett or Prowlarr)

Most people can skip this section.

[Jackett](https://github.com/Jackett/Jackett) and
[Prowlarr](https://github.com/Prowlarr/Prowlarr) are free programs you run on your
own computer or server. They search the sites you set up in them. To use one, turn
on **Your own search server** in the search settings, then fill in:

- **Server**: Jackett or Prowlarr
- **Address**: where it runs, e.g. `http://127.0.0.1:9117` (Jackett) or
  `http://127.0.0.1:9696` (Prowlarr)
- **API key**: shown on Jackett's dashboard, or under *Settings ▸ General* in
  Prowlarr

If you use a proxy in ztorrent, searches go through it too. A server on your own
computer (`127.0.0.1`) cannot be reached that way, so run it somewhere the proxy
can reach.

## Privacy

A search sends your search words, and your IP address, to every source you have
turned on, and to nothing else. See [PRIVACY.md](../PRIVACY.md).
