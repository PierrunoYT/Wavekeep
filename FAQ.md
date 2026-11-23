# Wavekeep FAQ

This FAQ describes Wavekeep 0.3.0. Use the headings to jump to what you need.

---

## Read this first

**Wavekeep is an unofficial tool.** It is not made, approved or supported by Deezer or Spotify.

- **It can get your account banned.** Downloading with software like this breaks Deezer's and Spotify's terms of service. They can warn, restrict, suspend or ban an account, and they can do it without notice. Spacing out downloads (see [Can I slow down downloads?](#can-i-slow-down-or-space-out-downloads)) makes the activity look more like normal listening, but it does not remove the risk.
- **You use it at your own risk.** Wavekeep comes with no warranty. The developer takes no responsibility for banned or restricted accounts, lost playlists, subscriptions or purchases, or anything else that happens because you used it. Do not use an account you cannot afford to lose.
- **Copyright is your responsibility.** Only download music you have the right to copy. See [Is Wavekeep legal to use?](#is-wavekeep-legal-to-use)

If you do not accept this, do not use Wavekeep.

---

## 🚀 Getting started

### What is Wavekeep?

A desktop app (Windows installers are published; macOS and Linux can be built from source) that searches Deezer, Spotify and SoundCloud and downloads tracks, albums and playlists with tags and cover art. It was called Deezy before version 0.3.0.

### How do I get my Deezer ARL token?

1. Log into [deezer.com](https://www.deezer.com) in your browser.
2. Open DevTools (`F12`) and select **Application** in Chromium-based browsers or **Storage** in Firefox.
3. Open **Cookies** → `https://www.deezer.com`.
4. Copy the complete value of the `arl` cookie.
5. Paste it into Wavekeep's Settings and click **Save & Login**.

Treat the ARL like a password. Do not share it, post it in screenshots, or include it in bug reports.

### Can I use Spotify without Deezer?

Yes. Connect Spotify in Settings (a browser login on Spotify's own page), then choose Spotify as the music source. You do not need a Deezer account or ARL. You can also paste a full `open.spotify.com` track, album, artist or playlist link, or a `spotify:` URI, into the music URL field.

### Do I need an account for SoundCloud?

No. Choose SoundCloud as the music source in Search, or paste a `soundcloud.com` link. Wavekeep uses the same anonymous access as the soundcloud.com web player. Tracks that SoundCloud only streams DRM-protected (many major-label releases) or only as a 30-second preview (Go+ or region-restricted) cannot be downloaded; Wavekeep says so instead of saving anything.

### Does Spotify work with a Free account?

No, the Spotify source needs **Spotify Premium**. The library Wavekeep uses to talk to Spotify (librespot) closes the whole program as soon as it connects to a Free account, and nothing appears in the log. In 0.3.0 Wavekeep therefore just closes right after the Spotify login. The next version checks your account type first and shows "Spotify Premium is required" instead. Deleting saved credentials or app data does not help; only a Premium account does. Deezer still works with a Free account.

### Can I paste a link to download?

Yes. The **music URL** field in Search accepts `deezer.com`, `open.spotify.com` and `soundcloud.com` links:

- Track links add one track.
- Album and playlist links add all their tracks.
- Artist links add the tracks of every album returned for that artist, after asking you to confirm the number of albums. SoundCloud profile links add the user's uploads (up to 500) after asking you to confirm the number of tracks.

Deezer links must be full `http(s)://` links with a numeric ID, such as `https://www.deezer.com/track/123456`, optionally with a locale like `/en/`. Short links and other domains are not supported.

---

## 🎵 Downloads & quality

### What quality can I download in?

**Deezer**

| Account type | Available qualities |
|---|---|
| Free | MP3 128 kbps |
| Premium | MP3 128 kbps, MP3 320 kbps, FLAC |

Free accounts are always limited to MP3 128 kbps, whatever you choose in Settings.

**Spotify** (Premium only, see [Does Spotify work with a Free account?](#does-spotify-work-with-a-free-account)): Settings offers Normal (96 kbps), High (160 kbps), Very high (320 kbps) and Lossless (FLAC). Each setting starts at its own bitrate and falls back to the others when Spotify does not supply it. At each bitrate OGG Vorbis is preferred over MP3. FLAC is only requested for Lossless; if a track offers none of the usual formats, Wavekeep takes whatever Spotify lists, such as AAC. Availability varies by track and region.

**SoundCloud**: there is no quality setting. Wavekeep downloads the best stream SoundCloud serves without encryption, today usually AAC 160 kbps (saved as `.m4a`), sometimes MP3 128 kbps. DRM-protected streams (Go+ quality and many major-label releases) are never used.

### Why did my Deezer download fall back to a lower quality?

Not every track exists in every quality. Wavekeep tries your selected Deezer quality first and then falls back: **FLAC → MP3 320 kbps → MP3 128 kbps** (MP3 320 falls back to MP3 128). Completed downloads show the requested and the actual quality when a fallback happened. Spotify picks the closest available format for its own setting.

### Where are my downloads saved?

In the folder chosen in Settings (by default a `Wavekeep` folder inside your Music folder). Folder structures: Flat, Artist/Track, Artist/Album/Track, Album/Track, or a custom template.

Custom templates support `{artist}`, `{album}`, `{title}`, `{track_number}`, `{disc_number}`, `{release_date}` and `{release_year}`, plus the aliases `{track}`, `{disc}` and `{year}`. Use `/` or `\` to make folders.

Slashes inside metadata, such as `AC/DC`, become underscores and never create folders. Invalid filename characters and Windows device names are cleaned, and file and folder names are shortened to 180 bytes (the extension is always kept). See the [template examples](README.md#custom-folder-templates).

### Can I download whole albums, playlists, Loved Tracks or my own playlists?

Yes. Click **Download All** on an album or playlist, or click its title to pick individual tracks (this also works for albums in an artist's discography). In Search, the **Loved Tracks** tab lists your favourites with **Download All**, and **My Playlists** lists your profile's playlists. This works for Deezer, including private playlists, and for Spotify. Pasted links to supported private playlists work too.

Wavekeep runs three downloads at the same time by default; change **Simultaneous Downloads** in Settings (1 to 10).

### Will Wavekeep download a track again if I already have it?

Not by default. With **Skip Existing Files** on, Wavekeep checks whether a supported audio file already exists at the path it would save to (Deezer: MP3 and FLAC; Spotify: OGG, FLAC, MP3 and M4A). A non-empty match is skipped and shown as "Already downloaded", even if you cleared the history.

Only the current folder structure is checked, so files saved under an earlier structure are not found. With the setting off, a second copy is saved with a numbered name such as `Artist - Title (1).mp3`.

### Can I slow down or space out downloads?

Yes, in Settings:

- **Pause Between Downloads** waits 2 to 60 seconds before each next download starts.
- **Download Speed Limit** caps the total speed of all downloads. The 1411 kbit/s (FLAC) and 320 kbit/s (MP3 320) presets download each track in about the time it takes to listen to it.

This is not protection against bans. See [Read this first](#read-this-first).

### Does Resume continue from the same byte position?

No. Pause cancels the transfer and removes its temporary file; Resume starts the track again from the beginning.

### Does clearing download history delete my music?

No. It removes inactive history records, not audio files. Active and queued downloads stay visible.

---

## 🔒 Privacy & security

### Can my account get banned?

Yes. See [Read this first](#read-this-first). Wavekeep never asks for your Deezer or Spotify password, but your ARL is a session credential: protect it like a password.

### What does Wavekeep collect or send?

**Nothing goes to the developer.** There is no analytics, telemetry, crash reporting, update check or Wavekeep server. Wavekeep only talks to Deezer, Spotify and SoundCloud (and their image and audio servers) to log in, search, preview and download. Your Spotify login happens in your browser on Spotify's own page.

| Stored on your computer | Where | How to remove it |
|---|---|---|
| Settings, incl. your last 20 searches | `settings.json` in the app-data folder | Clear or turn off search history in Settings |
| Download history | `download_history.json` | Clear it in Downloads |
| Custom themes | `themes/` | Delete them in Settings |
| Deezer ARL and Spotify logins | OS credential store (the ARL goes into `settings.json` only if that store is unavailable) | Log out or disconnect in Settings |
| Diagnostic log | `wavekeep-debug.log` in the logs folder | **Delete log** in Info & FAQ |
| Your music | Your download folder | Delete the files yourself; clearing history never touches them |

### How are my logins stored?

- **Deezer ARL:** in the OS credential store (Windows Credential Manager, macOS Keychain, Linux Secret Service). If that is unavailable and nothing is stored yet, it falls back to `settings.json` and moves into the credential store once it works. `WAVEKEEP_NO_KEYRING=1` forces the file. The window never gets the ARL back, a new ARL is checked with Deezer before it replaces the old one, and it is only sent to Deezer over HTTPS.
- **Spotify:** a browser login, not your password. The tokens go to the credential store, separately from the ARL; without it you log in again after a restart. **Disconnect** in Settings removes them.

### Does Wavekeep keep a log? What is in it?

Yes, a diagnostic log that never leaves your computer unless you share it. It is in `%LOCALAPPDATA%\com.pierr.wavekeep\logs` (Windows), `~/Library/Logs/com.pierr.wavekeep` (macOS) or `~/.local/share/com.pierr.wavekeep/logs` (Linux).

- **Records:** start and version, the move from Deezy, login results (account type only), quality fallbacks, each download's start and result, Spotify rate limits and errors, with track IDs.
- **Track names, artists and file paths** are included unless you turn off **Track Details in Log** in Settings; then they read `[hidden]`.
- **Never:** passwords, your ARL, login tokens or your searches. Your home folder shows as `~`, web addresses lose their query part, and long token-like strings become `[redacted]`.
- **Size:** at 1 MB it rotates to `wavekeep-debug.old.log`. **Delete log** in Info & FAQ removes both files.

### What local files can Wavekeep access?

Only the audio files and cover images you pick in its own file dialogs, plus audio files in your download folder. The Tag Editor keeps a hidden `.wavekeep.bak` copy while saving and restores the file if the save fails. Settings, history and themes live in the app-data folder.

---

## 🛠️ Troubleshooting

### Downloads are stalling or not starting.

- Check your internet connection.
- **Deezer:** save a current ARL, and try MP3 128 kbps if higher quality is unavailable.
- **Spotify:** disconnect and reconnect in Settings.
- Pause and resume the download, or retry it from the history.
- Fully exit Wavekeep from the tray and start it again.
- Press **Copy recent log** in Info & FAQ: it shows the exact error (see [How do I report a problem?](#how-do-i-report-a-problem)).

Interrupted downloads come back paused after a restart; queued tracks that had not started are not saved. A track can also be unavailable in your region. Quitting through Wavekeep cancels downloads and cleans up (at most 30 seconds); after a crash an unfinished `.wavekeep.part` file may remain, and you can delete it.

### My ARL expired or stopped working.

Log into Deezer in your browser, copy the current `arl` cookie and save it in Settings. Wavekeep already retries once by itself when it sees an expired session.

### Spotify search is rate-limited, or my Client ID does not work.

The default Client ID is shared by every librespot install, so its search quota is often used up, and Spotify currently blocks the fallback search. **Your own Client ID is the fix:**

1. Create an app in the Spotify Developer Dashboard and add `http://127.0.0.1:5588/login` as a redirect URI.
2. Paste its Client ID into **Settings → Spotify Client ID** (this disconnects you).
3. Reconnect and approve **both** browser logins: one for downloads (always the default ID) and one for search and your playlists.

Downloads do not depend on this quota. An app in Spotify's development mode can carry limits Wavekeep cannot see. `WAVEKEEP_SPOTIFY_CLIENT_ID` works as an override when the field is blank. If a login hangs, it gives up after 5 minutes and frees port 5588. From the next version, connecting also checks your account type through this quota; if it is used up, connecting fails with the rate-limit message until you set your own Client ID or try again later.

### A link, album or playlist does not load.

Use a full `https://` link from `deezer.com` (`/track/`, `/album/`, `/artist/` or `/playlist/` with a numeric ID); short links are not supported. A failed page while loading is reported as an error instead of a shorter list, so check your connection and retry. If it still fails, update your ARL and check that the content is available in your region.

### Covers or previews do not load.

Update to the latest release and allow HTTPS to `api.deezer.com`, `*.dzcdn.net`, `api.spotify.com`, `*.scdn.co` and `*.spotifycdn.com` in your firewall, DNS filter or proxy.

### The app shows a blank or black screen.

Exit Wavekeep from the tray and reopen it, or reinstall the latest release. As a last resort, close it and rename its app-data folder (`%APPDATA%\com.pierr.wavekeep` on Windows, `~/Library/Application Support/com.pierr.wavekeep` on macOS, `~/.local/share/com.pierr.wavekeep` on Linux). That resets settings, history and themes (and the log on Linux); logins in the OS credential store stay.

### How do I report a problem?

Open **Info & FAQ**, press **Copy recent log**, and paste it with a short description into a [GitHub issue](https://github.com/PierrunoYT/Wavekeep/issues) or the [Discord server](https://discord.gg/dvuWBeXSf3). **Open log folder** shows the whole file. The log never contains passwords, your ARL or login tokens, and you can read it before sharing; to leave out track names and paths, turn off **Track Details in Log**, press **Delete log**, reproduce the problem, then copy. See [Does Wavekeep keep a log?](#does-wavekeep-keep-a-log-what-is-in-it).

---

## 🔄 Updates & moving from Deezy

### How do I update Wavekeep?

There is no automatic updater, and Wavekeep never checks for updates. Download the latest `.exe` or `.msi` from the [GitHub Releases page](https://github.com/PierrunoYT/Wavekeep/releases/latest), fully exit Wavekeep from the tray, and run the installer. Settings and history should be kept, but back up anything important. The installers are not code-signed, so Windows SmartScreen may warn.

Prebuilt macOS and Linux packages are not published. If you built from source:

```bash
cd Wavekeep
git pull
cd deezy
bun install --frozen-lockfile
bun run tauri build
```

Then install the bundle from `src-tauri/target/release/bundle/`. See the [development guide](DEVELOPMENT.md).

### I used Deezy. Will I lose my data when I install Wavekeep?

No. Wavekeep is Deezy's new name. The first time it starts, it moves your settings, download history and custom themes from Deezy's data folder into its own, and moves your saved Deezer ARL and Spotify login to Wavekeep's entries in the credential store. Your download folder setting and your music files are not touched.

With the `.exe` installer, Deezy stays installed next to Wavekeep. **Start Wavekeep once before you uninstall Deezy.** If you uninstall Deezy first and tick **"Delete the application data"**, its uninstaller deletes the data before Wavekeep can move it. The `.msi` installer upgrades Deezy in place, so this does not come up.

### What exactly does the move from Deezy do?

Every Tauri app has an internal identifier that decides where it keeps its data and how its credential-store entries are named. Deezy's was `com.pierr.deezy`; Wavekeep's is `com.pierr.wavekeep`. On every start, before anything is loaded, Wavekeep:

- **Moves your data:** `settings.json`, `download_history.json` and the `themes` folder go from the old folder to the new one (on Windows from `%APPDATA%\com.pierr.deezy` to `%APPDATA%\com.pierr.wavekeep`). Anything already in the new folder is kept and never overwritten. The old folder is removed only once it is empty.
- **Moves your logins:** each saved credential (Deezer ARL, Spotify tokens) is copied to the new name and read back to check it, and only then is the old entry deleted, so a secret is never lost. If Wavekeep already has a newer login, the old one is just removed. If an old entry cannot be removed, it stays and the log says so. With `WAVEKEEP_NO_KEYRING` set, the ARL is in `settings.json` and moves with it.

Once everything has moved these steps do nothing. What happened is in Wavekeep's log; see [Does Wavekeep keep a log?](#does-wavekeep-keep-a-log-what-is-in-it).

Good to know:

- After the move, the old Deezy app starts with default settings and no saved logins.
- On macOS, the Keychain may ask once whether Wavekeep may read the logins Deezy saved.
- On Windows, Deezy's browser cache in `%LOCALAPPDATA%\com.pierr.deezy` is not moved; you can delete that folder.
- On Linux, the old `~/.local/share/com.pierr.deezy` folder keeps Deezy's browser data, so it is not removed automatically; you can delete it.
- The startup warning appears once more, because that choice was stored with the browser data.
- The older `DEEZY_NO_KEYRING` and `DEEZY_SPOTIFY_CLIENT_ID` environment variables still work next to the `WAVEKEEP_` ones.

---

## ⚖️ Legal

### Is Wavekeep legal to use?

Wavekeep is provided for educational and personal use, but that does not make every use lawful. Downloading or extracting music can violate [Deezer's](https://www.deezer.com/legal/cgu) or [Spotify's](https://www.spotify.com/legal/end-user-agreement/) terms of service, copyright law, or both. A subscription does not give you permission to copy or share recordings.

Laws differ by country. You alone are responsible for finding out whether your use is allowed and for getting any permission you need. This FAQ is not legal advice. Do not redistribute downloaded music or use it commercially without the rights holders' permission.

### Does Wavekeep support the artists?

No. Downloading through Wavekeep does not pay artists or other rights holders. Support them through authorised streams, purchases, merchandise or live shows.

---

## 💬 Community & support

Question not answered here? Join the [Discord server](https://discord.gg/dvuWBeXSf3) or open an [issue on GitHub](https://github.com/PierrunoYT/Wavekeep/issues). Remember: Wavekeep is unofficial, comes without warranty, and is used at your own risk.
