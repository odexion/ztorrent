# Privacy notice

**Short version: ztorrent collects nothing about you.** There are no accounts, no
analytics, no telemetry, no crash reports and no advertising. The ztorrent
project runs no servers that ztorrent talks to, and it receives no information
about you or your use of the app.

## What stays on your computer

ztorrent saves the following on your computer only:

- Your torrent list, labels, preferences, column layout and window position
- Download progress ("resume data"), so downloads can carry on after a restart
- Your search settings, including any search sources you added

On macOS and Windows, the proxy password and the Jackett/Prowlarr API key are
encrypted with your system's keychain before they are saved. On Linux they are
saved as plain text in the settings file, because there is no keychain ztorrent
can rely on there.

You can delete everything by removing ztorrent's settings folder:

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/ztorrent` |
| Windows | `%APPDATA%\ztorrent` |
| Linux | `~/.config/ztorrent` |

## What leaves your computer, and only when

ztorrent connects to other computers only to do what you ask of it. None of these
connections go to the ztorrent project.

| When | Where it connects | What the other side can see |
|---|---|---|
| You download or share a torrent | The torrent's trackers, other peers, and the DHT network | Your IP address and which torrents you are sharing. Everyone in a torrent sees this; that is how BitTorrent works. |
| Port forwarding is on (UPnP, on by default) | Your own router | A request to open a port |
| You add a torrent from a web link | The address you gave | Your IP address and the request |
| You search online | Each search source you have turned on: the Internet Archive, your Jackett or Prowlarr server, and any sources you added, with the lookup address of one that has one | Your search words and your IP address |
| Automatic updates are on (default) | GitHub, every few hours | Your IP address and your ztorrent version |

If you set a proxy, or bind ztorrent to a network interface (for example a VPN),
all of the above goes that way, or not at all. Searches and update checks go that
way too.

Turn things off in **Preferences**: updates under *General*, port forwarding and
peer discovery under *Connection*. Search sources can be turned off in **Search
Online ▸ Search settings**.

## Third parties

The sites and servers listed above, including any search source you add, have
their own privacy practices, which the ztorrent project does not control. Read
theirs before using them.

## Changes

If this notice changes, the change will be visible in this file's history at
<https://github.com/odexion/ztorrent/commits/main/PRIVACY.md>.
