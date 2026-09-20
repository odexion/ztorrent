#include "ztorrent-lt-sys/include/bridge.h"
#include "ztorrent-lt-sys/src/lib.rs.h"

#include <algorithm>
#include <chrono>
#include <filesystem>
#include <sstream>
#include <stdexcept>
#include <vector>

#include <libtorrent/alert_types.hpp>
#include <libtorrent/bdecode.hpp>
#include <libtorrent/announce_entry.hpp>
#include <libtorrent/bencode.hpp>
#include <libtorrent/create_torrent.hpp>
#include <libtorrent/entry.hpp>
#include <libtorrent/fingerprint.hpp>
#include <libtorrent/ip_filter.hpp>
#include <libtorrent/load_torrent.hpp>
#include <libtorrent/magnet_uri.hpp>
#include <libtorrent/peer_info.hpp>
#include <libtorrent/read_resume_data.hpp>
#include <libtorrent/session_params.hpp>
#include <libtorrent/session_stats.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/torrent_status.hpp>
#include <libtorrent/write_resume_data.hpp>

namespace ztlt {

namespace {

// A .torrent is read in full into memory; anything past this is not a torrent
// anyone meant to add, and the parser should not be asked to try.
constexpr int MAX_TORRENT_BYTES = 16 * 1024 * 1024;

std::string hex(lt::sha1_hash const& h) {
    std::ostringstream out;
    out << h;
    return out.str();
}

std::string info_hash_of(lt::info_hash_t const& ih) {
    if (ih.has_v1()) return hex(ih.v1);
    std::ostringstream out;
    out << ih.v2;
    return out.str();
}

bool is_udp(std::string const& url) {
    return url.size() >= 4 &&
        std::equal(url.begin(), url.begin() + 4, "udp:", [](char a, char b) {
            return std::tolower(static_cast<unsigned char>(a)) == b;
        });
}

lt::settings_pack make_pack(SessionConfig const& c) {
    namespace sp = lt;
    lt::settings_pack p;

    lt::alert_category_t mask = lt::alert_category::status | lt::alert_category::error |
        lt::alert_category::storage | lt::alert_category::port_mapping;
    p.set_int(lt::settings_pack::alert_mask, static_cast<int>(static_cast<std::uint32_t>(mask)));

    p.set_str(lt::settings_pack::listen_interfaces, std::string(c.listen_interfaces));
    p.set_str(lt::settings_pack::outgoing_interfaces, std::string(c.outgoing_interfaces));

    p.set_bool(lt::settings_pack::enable_dht, c.enable_dht);
    p.set_bool(lt::settings_pack::enable_lsd, c.enable_lsd);
    p.set_bool(lt::settings_pack::enable_upnp, c.enable_upnp);
    p.set_bool(lt::settings_pack::enable_natpmp, c.enable_upnp);
    p.set_bool(lt::settings_pack::enable_outgoing_utp, c.enable_utp);
    p.set_bool(lt::settings_pack::enable_incoming_utp, c.enable_utp && c.enable_incoming_tcp);
    p.set_bool(lt::settings_pack::enable_incoming_tcp, c.enable_incoming_tcp);

    int policy = c.encryption == 0 ? lt::settings_pack::pe_disabled
        : c.encryption == 2 ? lt::settings_pack::pe_forced
        : lt::settings_pack::pe_enabled;
    p.set_int(lt::settings_pack::out_enc_policy, policy);
    p.set_int(lt::settings_pack::in_enc_policy, policy);
    p.set_int(lt::settings_pack::allowed_enc_level, lt::settings_pack::pe_both);

    if (c.proxy_host.empty()) {
        p.set_int(lt::settings_pack::proxy_type, lt::settings_pack::none);
    } else {
        p.set_int(lt::settings_pack::proxy_type,
            c.proxy_username.empty() ? lt::settings_pack::socks5 : lt::settings_pack::socks5_pw);
        p.set_str(lt::settings_pack::proxy_hostname, std::string(c.proxy_host));
        p.set_int(lt::settings_pack::proxy_port, c.proxy_port);
        p.set_str(lt::settings_pack::proxy_username, std::string(c.proxy_username));
        p.set_str(lt::settings_pack::proxy_password, std::string(c.proxy_password));
        // Names are resolved at the proxy, and everything that can go through it does.
        p.set_bool(lt::settings_pack::proxy_hostnames, true);
        p.set_bool(lt::settings_pack::proxy_peer_connections, true);
        p.set_bool(lt::settings_pack::proxy_tracker_connections, true);
    }

    p.set_int(lt::settings_pack::download_rate_limit, std::max(0, c.download_rate));
    p.set_int(lt::settings_pack::upload_rate_limit, std::max(0, c.upload_rate));
    if (c.connections_limit > 0) p.set_int(lt::settings_pack::connections_limit, c.connections_limit);

    if (!c.user_agent.empty()) {
        p.set_str(lt::settings_pack::user_agent, std::string(c.user_agent));
        p.set_str(lt::settings_pack::handshake_client_version, std::string(c.user_agent));
    }
    if (!c.peer_fingerprint.empty()) p.set_str(lt::settings_pack::peer_fingerprint, std::string(c.peer_fingerprint));

    // The queue is ztorrent's own; libtorrent's automatic management is never used.
    p.set_int(lt::settings_pack::active_downloads, -1);
    p.set_int(lt::settings_pack::active_seeds, -1);
    p.set_int(lt::settings_pack::active_limit, -1);
    return p;
}

MetaInfo meta_from(lt::torrent_info const& ti, std::vector<std::string> const& trackers,
                   std::vector<std::string> const& url_seeds, std::vector<std::int64_t> const* progress,
                   std::vector<lt::download_priority_t> const* priorities) {
    MetaInfo m;
    m.has_metadata = true;
    m.name = ti.name();
    m.info_hash = info_hash_of(ti.info_hashes());
    m.comment = ti.comment();
    m.creator = ti.creator();
    m.created = static_cast<std::int64_t>(ti.creation_date());
    m.private_flag = ti.priv();
    m.piece_length = ti.piece_length();
    m.num_pieces = ti.num_pieces();
    m.total_size = ti.total_size();
    for (auto const& t : trackers) m.trackers.push_back(rust::String(t));
    for (auto const& u : url_seeds) m.url_seeds.push_back(rust::String(u));
    auto const& fs = ti.files();
    for (lt::file_index_t i : fs.file_range()) {
        FileEntry f;
        f.path = fs.file_path(i);
        f.name = std::string(fs.file_name(i));
        f.size = fs.file_size(i);
        auto idx = static_cast<std::size_t>(static_cast<int>(i));
        f.downloaded = progress && idx < progress->size() ? (*progress)[idx] : 0;
        f.priority = priorities && idx < priorities->size()
            ? static_cast<std::uint8_t>(static_cast<std::uint8_t>((*priorities)[idx]))
            : 4;
        f.pad = fs.pad_file_at(i);
        m.files.push_back(std::move(f));
    }
    return m;
}

} // namespace

// ------------------------------------------------------------------ session

Session::Session(SessionConfig const& config) {
    lt::session_params params;
    params.settings = make_pack(config);
    m_session = std::make_unique<lt::session>(std::move(params));
    m_dht_nodes_metric = lt::find_metric_idx("dht.dht_nodes");

    // libtorrent exempts peers on the local network from the rate limits by
    // putting them in a class of their own. WebTorrent throttled every peer,
    // and a limit the user set is a limit on everything: every address goes in
    // the global class, which is the one the limits apply to.
    lt::ip_filter classes;
    auto const global = 1u << static_cast<std::uint32_t>(lt::session::global_peer_class_id);
    classes.add_rule(lt::make_address("0.0.0.0"), lt::make_address("255.255.255.255"), global);
    classes.add_rule(lt::make_address("::"), lt::make_address("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"), global);
    m_session->set_peer_class_filter(classes);
}

std::unique_ptr<Session> new_session(SessionConfig const& config) {
    return std::make_unique<Session>(config);
}

void Session::apply_config(SessionConfig const& config) {
    m_session->apply_settings(make_pack(config));
}

lt::torrent_handle const* Session::find(std::uint64_t handle) const {
    auto it = m_handles.find(handle);
    if (it == m_handles.end() || !it->second.is_valid()) return nullptr;
    return &it->second;
}

std::uint64_t Session::id_of(lt::torrent_handle const& h) const {
    auto it = m_ids.find(h);
    return it == m_ids.end() ? 0 : it->second;
}

std::uint64_t Session::add(AddParams const& p) {
    lt::add_torrent_params atp;
    bool from_resume = false;
    if (!p.resume.empty()) {
        lt::error_code ec;
        atp = lt::read_resume_data(
            {reinterpret_cast<char const*>(p.resume.data()), static_cast<std::ptrdiff_t>(p.resume.size())}, ec);
        from_resume = !ec && (atp.ti || atp.info_hashes.has_v1() || atp.info_hashes.has_v2());
        if (!from_resume) atp = lt::add_torrent_params{};
    }
    if (!from_resume) {
        if (!p.torrent.empty()) {
            lt::load_torrent_limits limits;
            limits.max_buffer_size = MAX_TORRENT_BYTES;
            atp = lt::load_torrent_buffer(
                {reinterpret_cast<char const*>(p.torrent.data()), static_cast<std::ptrdiff_t>(p.torrent.size())},
                limits);
        } else if (!p.magnet.empty()) {
            atp = lt::parse_magnet_uri(std::string(p.magnet));
        } else {
            throw std::runtime_error("No torrent data");
        }
        for (auto const& r : p.renamed) {
            atp.renamed_files[lt::file_index_t(r.index)] = std::string(r.path);
        }
    }

    atp.save_path = std::string(p.save_path);
    if (!p.file_priorities.empty()) {
        atp.file_priorities.clear();
        for (auto v : p.file_priorities) atp.file_priorities.push_back(lt::download_priority_t(v));
    }

    if (p.strip_udp_trackers) {
        std::vector<std::string> trackers;
        std::vector<int> tiers;
        for (std::size_t i = 0; i < atp.trackers.size(); ++i) {
            if (is_udp(atp.trackers[i])) continue;
            trackers.push_back(atp.trackers[i]);
            tiers.push_back(i < atp.tracker_tiers.size() ? atp.tracker_tiers[i] : 0);
        }
        atp.trackers = std::move(trackers);
        atp.tracker_tiers = std::move(tiers);
    }

    atp.flags &= ~(lt::torrent_flags::auto_managed | lt::torrent_flags::duplicate_is_error);
    if (p.paused) atp.flags |= lt::torrent_flags::paused;
    else atp.flags &= ~lt::torrent_flags::paused;
    if (p.sequential) atp.flags |= lt::torrent_flags::sequential_download;
    else atp.flags &= ~lt::torrent_flags::sequential_download;
    if (p.disable_pex) atp.flags |= lt::torrent_flags::disable_pex;
    if (p.max_uploads > 0) atp.max_uploads = p.max_uploads;

    lt::torrent_handle h = m_session->add_torrent(std::move(atp));
    auto existing = m_ids.find(h);
    if (existing != m_ids.end()) return existing->second;
    std::uint64_t id = m_next++;
    m_handles.emplace(id, h);
    m_ids.emplace(h, id);
    return id;
}

bool Session::remove(std::uint64_t handle, bool delete_files) {
    auto it = m_handles.find(handle);
    if (it == m_handles.end()) return false;
    lt::torrent_handle h = it->second;
    m_ids.erase(h);
    m_handles.erase(it);
    if (!h.is_valid()) return false;
    m_session->remove_torrent(h, delete_files ? lt::session_handle::delete_files : lt::remove_flags_t{});
    return true;
}

void Session::pause(std::uint64_t handle) const {
    if (auto h = find(handle)) h->pause();
}

void Session::resume(std::uint64_t handle) const {
    if (auto h = find(handle)) h->resume();
}

void Session::recheck(std::uint64_t handle) const {
    if (auto h = find(handle)) h->force_recheck();
}

void Session::reannounce(std::uint64_t handle) const {
    if (auto h = find(handle)) {
        h->force_reannounce();
        h->force_dht_announce();
    }
}

void Session::set_sequential(std::uint64_t handle, bool on) const {
    if (auto h = find(handle)) {
        if (on) h->set_flags(lt::torrent_flags::sequential_download);
        else h->unset_flags(lt::torrent_flags::sequential_download);
    }
}

void Session::set_file_priority(std::uint64_t handle, std::int32_t index, std::uint8_t priority) const {
    if (auto h = find(handle)) h->file_priority(lt::file_index_t(index), lt::download_priority_t(priority));
}

void Session::set_max_uploads(std::uint64_t handle, std::int32_t slots) const {
    if (auto h = find(handle)) h->set_max_uploads(slots);
}

bool Session::add_tracker(std::uint64_t handle, rust::Str url) const {
    auto h = find(handle);
    if (!h) return false;
    h->add_tracker(lt::announce_entry(std::string(url)));
    return true;
}

void Session::connect_peer(std::uint64_t handle, rust::Str ip, std::uint16_t port) const {
    auto h = find(handle);
    if (!h) throw std::runtime_error("torrent is not running");
    lt::error_code ec;
    auto addr = lt::make_address(std::string(ip), ec);
    if (ec) throw std::runtime_error("not an IP address");
    h->connect_peer(lt::tcp::endpoint(addr, port));
}

void Session::rename_file(std::uint64_t handle, std::int32_t index, rust::Str path) const {
    if (auto h = find(handle)) h->rename_file(lt::file_index_t(index), std::string(path));
}

bool Session::save_resume_data(std::uint64_t handle) const {
    auto h = find(handle);
    if (!h) return false;
    h->save_resume_data(lt::torrent_handle::save_info_dict);
    return true;
}

Status Session::status(std::uint64_t handle) const {
    Status s{};
    auto h = find(handle);
    if (!h) return s;
    auto st = h->status(lt::torrent_handle::query_name);
    s.valid = true;
    s.state = static_cast<std::uint8_t>(st.state);
    s.paused = static_cast<bool>(st.flags & lt::torrent_flags::paused);
    s.has_metadata = st.has_metadata;
    s.progress = st.progress_ppm / 1000000.0;
    s.total = st.total;
    s.total_done = st.total_done;
    s.total_wanted = st.total_wanted;
    s.total_wanted_done = st.total_wanted_done;
    s.all_time_download = st.all_time_download;
    s.all_time_upload = st.all_time_upload;
    s.download_rate = st.download_payload_rate;
    s.upload_rate = st.upload_payload_rate;
    s.num_peers = st.num_peers;
    s.num_seeds = st.num_seeds;
    s.list_seeds = st.list_seeds;
    s.list_peers = st.list_peers;
    s.distributed_copies = st.distributed_copies;
    if (st.errc) s.error = st.errc.message();
    s.name = st.name;
    return s;
}

MetaInfo Session::meta(std::uint64_t handle) const {
    MetaInfo m{};
    auto h = find(handle);
    if (!h) return m;
    auto ti = h->torrent_file();
    if (!ti) {
        m.info_hash = info_hash_of(h->info_hashes());
        return m;
    }
    std::vector<std::string> trackers;
    for (auto const& ae : h->trackers()) trackers.push_back(ae.url);
    std::set<std::string> seeds = h->url_seeds();
    return meta_from(*ti, trackers, std::vector<std::string>(seeds.begin(), seeds.end()), nullptr, nullptr);
}

rust::Vec<FileEntry> Session::files(std::uint64_t handle) const {
    rust::Vec<FileEntry> out;
    auto h = find(handle);
    if (!h) return out;
    auto ti = h->torrent_file();
    if (!ti) return out;
    auto progress = h->file_progress(lt::torrent_handle::piece_granularity);
    auto priorities = h->get_file_priorities();
    auto m = meta_from(*ti, {}, {}, &progress, &priorities);
    for (auto& f : m.files) out.push_back(std::move(f));
    return out;
}

rust::Vec<TrackerInfo> Session::trackers(std::uint64_t handle) const {
    rust::Vec<TrackerInfo> out;
    auto h = find(handle);
    if (!h) return out;
    auto now = lt::time_point_cast<lt::seconds32>(lt::clock_type::now());
    for (auto const& ae : h->trackers()) {
        TrackerInfo t{};
        t.url = ae.url;
        t.tier = ae.tier;
        t.complete = -1;
        t.incomplete = -1;
        std::int64_t next = -1;
        for (auto const& ep : ae.endpoints) {
            for (auto const& ih : ep.info_hashes) {
                if (ih.updating) t.updating = true;
                if (ih.start_sent) t.contacted = true;
                if (ih.start_sent && !ih.last_error) t.working = true;
                if (ih.last_error) {
                    t.failed = true;
                    if (t.message.empty()) t.message = ih.last_error.message();
                } else if (!ih.message.empty() && t.message.empty()) {
                    t.message = ih.message;
                }
                t.complete = std::max(t.complete, ih.scrape_complete);
                t.incomplete = std::max(t.incomplete, ih.scrape_incomplete);
                if (ih.next_announce > now) {
                    auto secs = std::chrono::duration_cast<std::chrono::seconds>(ih.next_announce - now).count();
                    if (next < 0 || secs < next) next = secs;
                }
            }
        }
        t.next_announce_secs = next;
        out.push_back(std::move(t));
    }
    return out;
}

rust::Vec<PeerEntry> Session::peers(std::uint64_t handle) const {
    rust::Vec<PeerEntry> out;
    auto h = find(handle);
    if (!h) return out;
    std::vector<lt::peer_info> infos;
    h->get_peer_info(infos);
    for (auto const& p : infos) {
        // Connections still being made are not peers yet; the Electron build
        // listed only connected wires.
        if (p.flags & (lt::peer_info::connecting | lt::peer_info::handshake)) continue;
        PeerEntry e{};
        e.ip = p.ip.address().to_string();
        e.port = p.ip.port();
        e.client = p.client;
        e.progress = p.progress;
        e.down_rate = p.payload_down_speed;
        e.up_rate = p.payload_up_speed;
        e.total_download = p.total_download;
        e.total_upload = p.total_upload;
        e.interesting = static_cast<bool>(p.flags & lt::peer_info::interesting);
        e.choked = static_cast<bool>(p.flags & lt::peer_info::choked);
        e.remote_interested = static_cast<bool>(p.flags & lt::peer_info::remote_interested);
        e.remote_choked = static_cast<bool>(p.flags & lt::peer_info::remote_choked);
        e.seed = static_cast<bool>(p.flags & lt::peer_info::seed);
        e.utp = static_cast<bool>(p.flags & lt::peer_info::utp_socket);
        e.outgoing = static_cast<bool>(p.flags & lt::peer_info::outgoing_connection);
        e.web_seed = p.connection_type == lt::peer_info::web_seed || p.connection_type == lt::peer_info::http_seed;
        e.encrypted = static_cast<bool>(p.flags & (lt::peer_info::rc4_encrypted | lt::peer_info::plaintext_encrypted));
        out.push_back(std::move(e));
    }
    return out;
}

rust::Vec<std::uint8_t> Session::piece_map(std::uint64_t handle) const {
    rust::Vec<std::uint8_t> out;
    auto h = find(handle);
    if (!h) return out;
    auto st = h->status(lt::torrent_handle::query_pieces);
    if (!st.has_metadata) return out;
    int n = st.pieces.size();
    std::vector<std::uint8_t> map(static_cast<std::size_t>(n), 0);
    for (int i = 0; i < n; ++i) {
        if (st.pieces[lt::piece_index_t(i)]) map[static_cast<std::size_t>(i)] = 2;
    }
    for (auto const& q : h->get_download_queue()) {
        auto i = static_cast<std::size_t>(static_cast<int>(q.piece_index));
        if (i < map.size() && map[i] == 0) map[i] = 1;
    }
    out.reserve(map.size());
    for (auto v : map) out.push_back(v);
    return out;
}

rust::Vec<std::uint8_t> Session::torrent_bytes(std::uint64_t handle) const {
    rust::Vec<std::uint8_t> out;
    auto h = find(handle);
    if (!h) return out;
    auto ti = h->torrent_file();
    if (!ti) return out;
    // The info dictionary is copied byte for byte, so the info hash of the file
    // written is the info hash of the torrent -- whatever keys it carried.
    auto section = ti->info_section();
    lt::entry e(lt::entry::dictionary_t);
    e["info"] = lt::entry(lt::entry::preformatted_type(section.begin(), section.end()));
    auto trackers = h->trackers();
    if (!trackers.empty()) {
        e["announce"] = trackers.front().url;
        lt::entry::list_type tiers;
        int current = -1;
        for (auto const& ae : trackers) {
            if (ae.tier != current) {
                tiers.emplace_back(lt::entry::list_type{});
                current = ae.tier;
            }
            tiers.back().list().emplace_back(ae.url);
        }
        e["announce-list"] = tiers;
    }
    auto seeds = h->url_seeds();
    if (!seeds.empty()) {
        lt::entry::list_type urls;
        for (auto const& u : seeds) urls.emplace_back(u);
        e["url-list"] = urls;
    }
    if (!ti->comment().empty()) e["comment"] = ti->comment();
    if (!ti->creator().empty()) e["created by"] = ti->creator();
    if (ti->creation_date()) e["creation date"] = static_cast<std::int64_t>(ti->creation_date());
    std::vector<char> buf;
    lt::bencode(std::back_inserter(buf), e);
    out.reserve(buf.size());
    for (char c : buf) out.push_back(static_cast<std::uint8_t>(c));
    return out;
}

rust::String Session::magnet_uri(std::uint64_t handle) const {
    auto h = find(handle);
    if (!h) return rust::String();
    return rust::String(lt::make_magnet_uri(*h));
}

bool Session::wait_for_alert(std::int32_t millis) {
    return m_session->wait_for_alert(lt::milliseconds(millis)) != nullptr;
}

rust::Vec<Alert> Session::pop_alerts() {
    rust::Vec<Alert> out;
    std::vector<lt::alert*> alerts;
    m_session->pop_alerts(&alerts);
    auto push = [&](AlertKind kind, lt::torrent_handle const* h, std::string message = {}, std::int32_t index = -1,
                    std::int64_t number = 0) {
        Alert a{kind, h ? id_of(*h) : 0, index, number, rust::String(message), rust::Vec<std::uint8_t>()};
        out.push_back(std::move(a));
    };
    for (lt::alert* a : alerts) {
        if (auto* x = lt::alert_cast<lt::metadata_received_alert>(a)) {
            push(AlertKind::MetadataReceived, &x->handle);
        } else if (auto* x = lt::alert_cast<lt::torrent_finished_alert>(a)) {
            push(AlertKind::TorrentFinished, &x->handle);
        } else if (auto* x = lt::alert_cast<lt::torrent_error_alert>(a)) {
            push(AlertKind::TorrentError, &x->handle, x->error.message());
        } else if (auto* x = lt::alert_cast<lt::file_error_alert>(a)) {
            push(AlertKind::FileError, &x->handle, x->error.message() + " (" + x->filename() + ")");
        } else if (auto* x = lt::alert_cast<lt::save_resume_data_alert>(a)) {
            Alert r{AlertKind::ResumeData, id_of(x->handle), -1, 0, rust::String(), rust::Vec<std::uint8_t>()};
            auto buf = lt::write_resume_data_buf(x->params);
            r.data.reserve(buf.size());
            for (char c : buf) r.data.push_back(static_cast<std::uint8_t>(c));
            out.push_back(std::move(r));
        } else if (auto* x = lt::alert_cast<lt::save_resume_data_failed_alert>(a)) {
            push(AlertKind::ResumeDataFailed, &x->handle, x->error.message());
        } else if (auto* x = lt::alert_cast<lt::file_renamed_alert>(a)) {
            push(AlertKind::FileRenamed, &x->handle, x->new_name(), static_cast<int>(x->index));
        } else if (auto* x = lt::alert_cast<lt::file_rename_failed_alert>(a)) {
            push(AlertKind::FileRenameFailed, &x->handle, x->error.message(), static_cast<int>(x->index));
        } else if (auto* x = lt::alert_cast<lt::torrent_checked_alert>(a)) {
            push(AlertKind::TorrentChecked, &x->handle);
        } else if (auto* x = lt::alert_cast<lt::listen_succeeded_alert>(a)) {
            push(AlertKind::ListenSucceeded, nullptr, x->address.to_string(), static_cast<int>(x->socket_type), x->port);
        } else if (auto* x = lt::alert_cast<lt::listen_failed_alert>(a)) {
            push(AlertKind::ListenFailed, nullptr, x->message());
        } else if (auto* x = lt::alert_cast<lt::torrent_removed_alert>(a)) {
            push(AlertKind::TorrentRemoved, nullptr, info_hash_of(x->info_hashes));
        } else if (auto* x = lt::alert_cast<lt::torrent_deleted_alert>(a)) {
            push(AlertKind::TorrentDeleted, nullptr, info_hash_of(x->info_hashes));
        } else if (auto* x = lt::alert_cast<lt::torrent_delete_failed_alert>(a)) {
            push(AlertKind::TorrentDeleteFailed, nullptr, x->error.message());
        } else if (lt::alert_cast<lt::dht_bootstrap_alert>(a)) {
            push(AlertKind::DhtBootstrap, nullptr);
        } else if (auto* x = lt::alert_cast<lt::portmap_error_alert>(a)) {
            push(AlertKind::PortmapError, nullptr, x->message());
        } else if (auto* x = lt::alert_cast<lt::session_stats_alert>(a)) {
            if (m_dht_nodes_metric >= 0) {
                auto counters = x->counters();
                if (m_dht_nodes_metric < static_cast<int>(counters.size())) m_dht_nodes = counters[m_dht_nodes_metric];
            }
        }
    }
    return out;
}

void Session::post_session_stats() {
    m_session->post_session_stats();
}

std::int64_t Session::dht_nodes() const {
    return m_dht_nodes;
}

bool Session::dht_running() const {
    return m_session->is_dht_running();
}

std::uint16_t Session::listen_port() const {
    return m_session->listen_port();
}

void Session::reopen_network_sockets() {
    m_session->reopen_network_sockets();
}

// ------------------------------------------------------------------ helpers

MetaInfo inspect_buffer(rust::Slice<std::uint8_t const> buffer) {
    lt::load_torrent_limits limits;
    limits.max_buffer_size = MAX_TORRENT_BYTES;
    auto atp = lt::load_torrent_buffer(
        {reinterpret_cast<char const*>(buffer.data()), static_cast<std::ptrdiff_t>(buffer.size())}, limits);
    if (!atp.ti) throw std::runtime_error("no torrent metadata in the file");
    auto m = meta_from(*atp.ti, atp.trackers, atp.url_seeds, nullptr, nullptr);
    // libtorrent 2.1 lifts the header fields out of torrent_info when it loads a
    // file, so read them from the file itself.
    lt::error_code ec;
    lt::bdecode_node root = lt::bdecode(
        {reinterpret_cast<char const*>(buffer.data()), static_cast<std::ptrdiff_t>(buffer.size())}, ec);
    if (!ec && root.type() == lt::bdecode_node::dict_t) {
        auto comment = root.dict_find_string_value("comment.utf-8");
        if (comment.empty()) comment = root.dict_find_string_value("comment");
        auto creator = root.dict_find_string_value("created by.utf-8");
        if (creator.empty()) creator = root.dict_find_string_value("created by");
        m.comment = std::string(comment);
        m.creator = std::string(creator);
        m.created = root.dict_find_int_value("creation date", 0);
    }
    return m;
}

MetaInfo inspect_magnet(rust::Str uri) {
    auto atp = lt::parse_magnet_uri(std::string(uri));
    MetaInfo m{};
    m.name = atp.name;
    m.info_hash = info_hash_of(atp.info_hashes);
    for (auto const& t : atp.trackers) m.trackers.push_back(rust::String(t));
    for (auto const& u : atp.url_seeds) m.url_seeds.push_back(rust::String(u));
    return m;
}

rust::Vec<std::uint8_t> create_torrent(CreateParams const& params, ProgressSink const& sink) {
    std::string input(params.input);
    namespace fs = std::filesystem;
    if (!fs::exists(input)) throw std::runtime_error("the file or folder to share does not exist");

    // Hidden files and the usual OS droppings stay out, as create-torrent kept them out.
    auto files = lt::list_files(input, [](std::string const& p) {
        auto name = fs::path(p).filename().string();
        return !(name.size() > 1 && name[0] == '.') && name != "Thumbs.db" && name != "desktop.ini";
    });
    lt::create_torrent t(std::move(files), params.piece_size, lt::create_torrent::v1_only);

    for (std::size_t i = 0; i < params.trackers.size(); ++i) {
        int tier = i < params.tiers.size() ? params.tiers[i] : static_cast<int>(i);
        t.add_tracker(std::string(params.trackers[i]), tier);
    }
    for (auto const& w : params.web_seeds) t.add_url_seed(std::string(w));
    if (!params.comment.empty()) t.set_comment(std::string(params.comment).c_str());
    if (!params.creator.empty()) t.set_creator(std::string(params.creator).c_str());
    t.set_priv(params.private_flag);

    int total = t.num_pieces();
    lt::set_piece_hashes(t, fs::path(input).parent_path().string(), [&](lt::piece_index_t i) {
        sink.report(static_cast<int>(i) + 1, total);
    });

    auto buf = t.generate_buf();
    rust::Vec<std::uint8_t> out;
    out.reserve(buf.size());
    for (char c : buf) out.push_back(static_cast<std::uint8_t>(c));
    return out;
}

} // namespace ztlt
