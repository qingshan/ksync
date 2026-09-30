# Library cover disappearance (device test, 2026-09-28)

The shared SH_Integration 4.1.0 launcher calls `utimes(filePath, NULL)` and
requests `com.lab126.scanner doFullScan` on every cover launch. Its extractor
handles `SCANNER_UPDATE` by deleting the old entry and inserting a new UUID.
The Library can display the intervening deletion after the WAF closes.
Removing the launch delay or changing WAF Back handling does not prevent it.

The local checkout `../sh_integration` adds an opt-in `# KeepLibraryEntry`
header, within the first six script lines. For marked scripts the launcher
skips both the timestamp update and the full scan. Unmarked scripts keep
existing behavior. Both ksync and tailscale scriptlets now include the marker.
Stock SH_Integration ignores it; package changes alone are not sufficient.

The test Kindle uses `/var/local/ksync-launcher/sh_integration_launcher` as
the appreg command for `tech.hackerdude.shell_integration.launcher`.
The original `/var/local/kmc/bin/sh_integration_launcher` is untouched.
The original registration is recorded in
`/var/local/ksync-launcher/original-command.txt`. A Hotfix update can reset
this registration; this is a local test deployment, not a published fix.

To restore the original launcher over SSH:

```sh
sqlite3 /var/local/appreg.db "UPDATE properties SET value='/var/local/kmc/bin/sh_integration_launcher' WHERE handlerId='tech.hackerdude.shell_integration.launcher' AND name='command';"
```

The marker is harmless with the original launcher. Script backups are at
`/mnt/us/ksync/var/ksync.sh.before-keep-entry` and
`/mnt/us/tailscale/var/tailscale.sh.before-keep-entry`.

Validation: the launcher regression checks preserve default refresh behavior,
skip both refresh operations for the exact marker, reject a partial marker,
and reset the choice for a subsequent ordinary script. Existing header and
command-generation tests pass. On Kindle, ksync launched from its Library
cover and closed with the header X; six post-close samples kept the same UUID,
and framebuffer captures retained all five Uncollected covers.

Tailscale also retained its UUID and cover in five post-close checks.
Evidence is in `target/library-entry-check/`.

## Avoiding Home during app startup

The scriptlets now wait for their detached KPM process, then poll appmgrd's
`activeApp` until the WAF becomes active (at most 15 polls, one second apart).
KPM's successful `start` call only queues activation, so waiting for KPM alone
still lets the shell launcher stop itself before the WAF becomes active.
A failed KPM launch exits immediately with its failure status.

The local shared launcher records the `pause` callback as a handoff and skips
its own `stop` and `xrefresh` after the script exits during that handoff.
The detached KPM process survives the shell launcher's normal unload.
Both apps' device logs verified the same flow. Tailscale's log showed `KPP_LIBRARY -> shell launcher -> tailscale ->
KPP_LIBRARY`, without a `KPP_HOME` transition. Its header X restored the same
collection. Scriptlet checks cover delayed activation, startup failure and the
bounded activation timeout. The original app-launch delay was not restored.

The previous cover-only launcher is saved on the Kindle as
`/var/local/ksync-launcher/sh_integration_launcher.before-handoff`, and the
pre-handoff scripts are saved in each app's `var/<app>.sh.before-handoff`.

## Intermittent Opening badge (2026-09-29)

Observed Tailscale's cover showing Opening while appmgrd reported KPPMainApp
and no shell-launcher process remained. Retapping that cover launched Tailscale;
closing it cleared the badge. Three further cover-launch/close cycles succeeded.
No new launcher change was applied for this report. The missing-savecontext
warnings also occur on successful launches; they do not yet establish the cause.
A permanent fix for the intermittent Opening state is not verified.

Ksync also showed the stuck badge while Library was active and no launch lock
existed. Retapping recovered it. Three launch/close cycles, including recovery,
returned to Uncollected with all five covers visible and the badge cleared.
This confirms recovery, not permanent prevention; no launcher code changed.

### Rapid reopen reproduction

At 09:31:04 ksync launched; it closed at 09:31:07. The Tailscale tap at
09:31:08 emitted Library tap metrics, but no native ExecuteAction or launcher
start followed. Retapping later succeeded. Reproduced by launching ksync,
closing after 1.2 seconds, and tapping Tailscale after another 1.2 seconds.
Library stayed active with Tailscale marked Opening.

Read-only disassembly of this device's KPPMainApp.js.hbc shows
ExclusiveBookOpenUtils.checkAndOpenBook sets _bookSwitching and schedules
its reset after 10000 ms. ExclusiveBookOpenCheck drops another open while
that flag is set. The app-start handler resets it early for reader/notebook
apps, not these WAFs. This explains the rapid-reopen reproduction and why
spaced launch tests passed. Waiting at least ten seconds after the previous
cover launch, then retapping, recovers it. No KPP firmware changes applied;
a permanent integration fix remains outstanding.
