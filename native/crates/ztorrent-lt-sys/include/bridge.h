#pragma once

#include "rust/cxx.h"

#include <cstdint>
#include <memory>
#include <unordered_map>

#include <libtorrent/session.hpp>
#include <libtorrent/torrent_handle.hpp>

namespace ztlt {

struct SessionConfig;
struct AddParams;
struct Status;
struct FileEntry;
struct TrackerInfo;
struct PeerEntry;
struct MetaInfo;
struct Alert;
struct CreateParams;
struct ProgressSink;

class Session {
public:
    explicit Session(SessionConfig const& config);

    void apply_config(SessionConfig const& config);

    std::uint64_t add(AddParams const& params);
    bool remove(std::uint64_t handle, bool delete_files);
    void pause(std::uint64_t handle) const;
    void resume(std::uint64_t handle) const;
    void recheck(std::uint64_t handle) const;
    void reannounce(std::uint64_t handle) const;
    void set_sequential(std::uint64_t handle, bool on) const;
    void set_file_priority(std::uint64_t handle, std::int32_t index, std::uint8_t priority) const;
    void set_max_uploads(std::uint64_t handle, std::int32_t slots) const;
    bool add_tracker(std::uint64_t handle, rust::Str url) const;
    void connect_peer(std::uint64_t handle, rust::Str ip, std::uint16_t port) const;
    void rename_file(std::uint64_t handle, std::int32_t index, rust::Str path) const;
    bool save_resume_data(std::uint64_t handle) const;

    Status status(std::uint64_t handle) const;
    MetaInfo meta(std::uint64_t handle) const;
    rust::Vec<FileEntry> files(std::uint64_t handle) const;
    rust::Vec<TrackerInfo> trackers(std::uint64_t handle) const;
    rust::Vec<PeerEntry> peers(std::uint64_t handle) const;
    rust::Vec<std::uint8_t> piece_map(std::uint64_t handle) const;
    rust::Vec<std::uint8_t> torrent_bytes(std::uint64_t handle) const;
    rust::String magnet_uri(std::uint64_t handle) const;

    bool wait_for_alert(std::int32_t millis);
    rust::Vec<Alert> pop_alerts();
    void post_session_stats();
    std::int64_t dht_nodes() const;
    bool dht_running() const;
    std::uint16_t listen_port() const;
    void reopen_network_sockets();

private:
    lt::torrent_handle const* find(std::uint64_t handle) const;
    std::uint64_t id_of(lt::torrent_handle const& h) const;

    std::unique_ptr<lt::session> m_session;
    std::unordered_map<std::uint64_t, lt::torrent_handle> m_handles;
    std::unordered_map<lt::torrent_handle, std::uint64_t> m_ids;
    std::uint64_t m_next = 1;
    std::int64_t m_dht_nodes = 0;
    int m_dht_nodes_metric = -1;
};

std::unique_ptr<Session> new_session(SessionConfig const& config);
MetaInfo inspect_buffer(rust::Slice<std::uint8_t const> buffer);
MetaInfo inspect_magnet(rust::Str uri);
rust::Vec<std::uint8_t> create_torrent(CreateParams const& params, ProgressSink const& sink);

} // namespace ztlt
