# Writing a `curl … | sh` installer

A reusable guide for shipping any GitHub-released app — CLI, desktop, or app
bundle — as a one-line install. Language- and framework-agnostic; the only
assumptions are POSIX `sh`, `curl` or `wget`, and releases with attached assets.

A working, tested implementation of everything below is in
[`installer-template.sh`](installer-template.sh). Adapting it means editing
three things: `APP`/`REPO`, `asset_candidates()`, and `install_artifact()`.

---

## 1. The contract

The one-liner is the front door. Most people meet the app through it, before
they have any reason to trust you, so the script has to earn that in one run.

```sh
curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/install.sh | sh
```

**Guarantees worth making, in the header comment where people who pipe-to-shell
can read them first:**

| Guarantee | Why |
|---|---|
| No `sudo`, ever | A script that asks for root is a script nobody reads twice |
| Nothing written outside `$HOME` unless a system path is already writable | `/Applications` if writable, `~/Applications` otherwise |
| Idempotent | Re-running upgrades in place; it is also the upgrade path |
| Overridable by env, not by flags | Works when piped, where `$@` cannot reach it |
| One temp dir, removed on any exit | `trap cleanup EXIT INT TERM` |

**Standard env surface:** `<APP>_VERSION` (a tag, default latest), `<APP>_REPO`,
`<APP>_PREFIX`, `GITHUB_TOKEN` (rate limit only), `NO_COLOR`.

> `curl … | sh` gives the script no stdin and no `$@`. Anything you would have
> made a flag has to be an environment variable, and anything you would have
> prompted for has to have a default.

---

## 2. Pipeline

Nine stages, each of which can fail cleanly:

```
ui_init → pick_downloader → detect_platform → mktemp -d
        → load_release → find_asset → download → install → report
```

Fail *before* downloading wherever possible: the missing-asset check costs one
string match and saves a 100MB transfer that ends in an error.

---

## 3. Portability rules

**Target POSIX `sh`, and test on bash 3.2.** macOS still ships bash 3.2 as
`/bin/sh` (GPLv3 is why it never moved). No arrays, no `local`, no `[[ ]]`, no
`${var,,}`.

**Keep the file pure ASCII.** bash 3.2 scans identifiers by the locale's
character rules, so a multi-byte character sitting next to `$VAR` in the *source*
can be read as part of its name: `$ARCH…` expands as one unset name, and under
`set -u` the script aborts. Bracing the variable fixes one line; ASCII removes
the whole class in every locale. Build every glyph from octal escapes instead:

```sh
G_FILL=$(printf '\342\224\201')   # U+2501 heavy horizontal
G_OK=$(printf '\342\234\223')     # U+2713 check
```

UTF-8 still reaches the terminal — it just is not in the file.

**Call system binaries by absolute path when the name is ambiguous.**
`/usr/bin/xattr`, not `xattr`: the PyPI package of that name installs an
executable taking different arguments, and the failure mode is a usage error
where the user expected a working app. Same reasoning applies to `python`,
`realpath`, `sed` (BSD vs GNU) — either pin the path or avoid the dependency.

**Force `LC_ALL=C` around anything that formats numbers.** Under a comma-decimal
locale, `awk` prints `9,0 MB`. It also puts `awk` in byte mode, which is what
repeating a multi-byte bar glyph needs.

**Validate every captured number before using it in arithmetic:**

```sh
case "$n" in ''|*[!0-9]*) n=0 ;; esac
```

**Fetch once into globals, not in a command substitution.** `TAG=$(load_release)`
runs the function in a subshell, where every other global it set is lost.

---

## 4. `set -eu` hazards

These two cost real debugging time and are invisible in a successful run.

**Subshells inherit `errexit`.** A backgrounded download whose `curl` exits
non-zero dies *before* writing its status file, and the poll waiting on that
file spins until killed. Every 404 becomes a hang. Disable it inside the child:

```sh
( set +e; curl -fL -sS -o "$dest" "$url" 2>"$err_file"; code=$?
  printf '%s' "$code" > "$WORK/dl.tmp"; mv -f "$WORK/dl.tmp" "$status_file" ) &
```

Write-then-rename so the poller never reads a half-written status.

**Reaping a killed child returns 128+SIGTERM**, which `errexit` turns into an
exit — and if you are inside the `EXIT` trap, cleanup stops there and the temp
directory survives. Swallow it explicitly:

```sh
kill "$SPIN_PID" 2>/dev/null || true
wait "$SPIN_PID" 2>/dev/null || true
return 0
```

**Poll a status file, not `kill -0`.** `kill -0` keeps returning true for an
exited-but-unreaped child.

**A trailing `[ … ] && cmd` as the last line of a function returns that test's
status.** Under `set -e` a false test then fails the caller. End such functions
with `return 0`.

---

## 5. Terminal output

**Detect capabilities separately; they are not one switch.**

```sh
TTY=0;   [ -t 1 ] && TTY=1
COLOR=0; [ "$TTY" = 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ] && COLOR=1
UNI=0;   case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in *[Uu][Tt][Ff]*8*) [ "$TTY" = 1 ] && UNI=1 ;; esac
CR="";      [ "$TTY" = 1 ] && CR=$(printf '\r')
CLR_EOL=""; [ "$TTY" = 1 ] && CLR_EOL="${ESC}[K"
```

Note that clear-to-EOL follows `TTY`, not `COLOR`: a shrinking line leaves
residue without it, `NO_COLOR` or not. And a bare `\r` piped into a CI log is
just a stray byte in the middle of the transcript.

**The degradation ladder — each step falls back one notch, not all the way:**

| Condition | Effect |
|---|---|
| Full UTF-8 tty | Heavy-rule bar, braille spinner, colour, in-place redraw |
| Non-UTF-8 locale | `====>----` bar, `\| / - \` spinner, everything else kept |
| `NO_COLOR` or `TERM=dumb` | No SGR codes; bar and redraw kept |
| Not a tty (pipe, CI) | Plain lines, no redraw, no animation |
| `sleep 0.08` unsupported | No animation — a once-a-second spinner is worse than none |

**Draw your own progress bar** if you care what it looks like: `curl
--progress-bar` cannot be styled. Run the transfer in the background and render
from the size of the file as it lands — `wc -c < "$dest"`, which needs no
cooperation from the downloader and works identically for `wget`.

**One process per frame.** At a dozen frames a second the process count is the
cost that shows: build the whole line in a single `awk`, not a pipeline of
`cut`/`bc`/`printf`.

**Size the bar to the terminal**, with floor and ceiling:

```sh
COLS=$(tput cols 2>/dev/null || stty size 2>/dev/null | cut -d' ' -f2 || echo 80)
BAR_W=$((COLS - 54)); [ "$BAR_W" -gt 40 ] && BAR_W=40; [ "$BAR_W" -lt 12 ] && BAR_W=12
```

**Hide the cursor while animating, and show it again from the `EXIT` trap** —
including on Ctrl-C, or you leave the user's terminal without a cursor.

**Draw one final frame at 100%** before the finished line, or the bar ends
wherever the last poll happened to land.

**Say what happened, once, per step.** A spinner line is replaced by a
`✓ installed to /path` line — the transcript afterwards should read as a short
list of facts, not a scroll of progress frames.

---

## 6. Resolving the right asset

**Read the URL off the release; never construct it.** Artifact names carry the
version, so the tag alone is not enough.

**Anchor the match to the end of the name.** A bare `.dmg` also matches the
`.dmg.blockmap` published beside it; `.tar.gz` matches `.tar.gz.sha256`. Match on
suffix, and take the first hit:

```sh
case "$u" in *"$1") printf '%s\n' "$u"; break ;; esac
```

**Try every architecture spelling, not one.** Packagers each use their own
convention — the same build is `x64` in one artifact, `x86_64` in the next and
`amd64` in the third. Normalise `uname -m`, then expand back out to aliases when
matching.

**Handle Rosetta.** A shell under Rosetta reports `x86_64` on Apple silicon:

```sh
[ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = 1 ] && ARCH=arm64
```

**Take the size from the release JSON** before spending a `HEAD` request on it;
fall back to `Content-Length`, and to a spinner with no bar when neither is
known.

**Parse JSON without `jq`** — you cannot depend on it being installed. `grep -o`
for the quoted field is enough for `tag_name` and `browser_download_url`; for
positional fields like `size`, split on commas so each field lands on its own
line and the parse holds whether or not the API pretty-prints.

**Support `GITHUB_TOKEN` for the API call.** Anonymous API access is limited to
60 requests/hour per IP, which CI and shared networks hit routinely.

---

## 7. Installing and reporting

**Prefer a system path only when it is already writable**, and fall back to the
home directory rather than escalating.

**Check `PATH` and say so.** Installing into `~/.local/bin` and not mentioning it
is not on `PATH` produces a "command not found" the user blames on you:

```sh
case ":$PATH:" in *":$PREFIX:"*) ... ;; *) warn "add it to your profile" ;; esac
```

**Predict the first-run failure and pre-empt it.** If the artifact needs a
runtime library, check for it at install time and name the package — a warning
during install beats a silent failure at launch.

**Register desktop integration on Linux** (`.desktop` entry, MIME/scheme
handlers, icon) as best-effort: never abort an otherwise good install over it.

**Clear quarantine on unsigned macOS builds**, and document the right-click ▸
Open path in the README as expected rather than alarming.

**End with the command to run**, not with a wall of notes.

---

## 8. Release-side requirements

The installer is one half of a contract; CI is the other.

- **Name artifacts deterministically**, with version, OS and arch — and set the
  name explicitly rather than accepting a packager default that contains spaces.
- **One source of truth for the naming scheme.** If an in-app updater also picks
  artifacts, it and the installer must agree; say so in a comment in both.
- **Put the one-liner at the top of every release's notes.** Most people arrive
  at a release page from a link, not from the README, and the notes must stand
  on their own. Generate it in CI, and re-apply it to releases created by hand or
  by an earlier attempt at the same tag.
- **Upload into an existing release rather than failing on it.** `gh release
  create` refuses a tag that already has one, which strands the installers as
  run artifacts that expire.
- **`if-no-files-found: error`**, and assert the release directory is non-empty
  before publishing. A silent empty release is worse than a failed job.
- **Version README download links by tag, not `/releases/latest/`.** When names
  carry the version, a `latest/` link 404s the day the next release lands; a tag
  link only goes stale.

---

## 9. Test matrix

The bugs are all in the paths that fail, so test those first: **a 404, a dropped
connection, a missing asset, a bad tag.** A successful install exercises none of
the code that goes wrong.

```sh
sh -n install.sh && bash -n install.sh     # bash 3.2 syntax too
shellcheck -s sh install.sh

sh install.sh                              # non-tty: plain lines
LC_ALL=en_US.UTF-8 <pty> sh install.sh     # full: bar, spinner, colour
NO_COLOR=1 LC_ALL=C <pty> sh install.sh    # ASCII bar, no colour
LC_ALL=tr_TR.UTF-8 <pty> sh install.sh     # comma-decimal locale
TERM=dumb /bin/bash install.sh             # bash 3.2, no animation
APP_VERSION=v0.0.0-nope sh install.sh      # bad tag  -> clean error
                                           # bogus asset URL -> fails in ~1s, no hang
```

Where `<pty>` is needed and `script` is unavailable (it refuses when stdout is
not a terminal), a five-line Python harness does it:

```python
import os, pty, sys
out = []
pty.spawn(sys.argv[1:], lambda fd: (lambda d: (out.append(d), d)[1])(os.read(fd, 1024)))
open('pty.out', 'wb').write(b''.join(out))
```

**Verify what you cannot see:** count `ESC[K` occurrences to prove clear-to-EOL
is emitted, count `ESC[36m` to prove `NO_COLOR` suppressed colour, and grep the
captured frames for `.` vs `,` under a comma-decimal locale.

---

## 10. Known limits of the template

- `date +%s` has one-second resolution, so a sub-second download reports
  `0 B/s`. Fixing it means a non-POSIX time source; the rate is decoration.
- No checksum or signature verification. If your releases publish checksums, add
  a verify step between download and install — it is the one thing this design
  leaves on the table.
- GitHub-specific release JSON. Another forge means changing `load_release()`
  and `asset_url()`; nothing else knows where the URL came from.
