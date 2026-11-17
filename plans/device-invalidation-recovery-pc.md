# Device-loss recovery: finish the fix on the PC

The fix is on branch `fix/device-invalidation-recovery`. It has never been
compiled: the notebook that wrote it has no MSVC linker and too little disk
space for one. This guide builds and tests it on the PC, then moves the
finished program back to the notebook.

## What the fix does

A recording on 2026-11-13 ran for 35 minutes but saved only 3. The audio
device dropped about 3 minutes in (error `0x88890004`,
`AUDCLNT_E_DEVICE_INVALIDATED`). The capture threads stopped on that error, and
nothing noticed until you pressed stop.

Now the recorder rebuilds the audio connection when the device drops, records
silence while the device is gone, and reports the drop and the recovery as
it happens.

## 1. Set up the PC (one time, about 10 GB)

Install these, in PowerShell:

```powershell
winget install Git.Git
winget install Rustlang.Rustup
winget install Gyan.FFmpeg
winget install Microsoft.VisualStudio.2022.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Close and reopen PowerShell, then check:

```powershell
rustup default stable-x86_64-pc-windows-msvc
ffmpeg -version
```

FFmpeg must be on PATH: the recorder and the sync tests both call it.

Install Claude Code if the PC does not have it.

## 2. Get the code

```powershell
git clone https://github.com/EricOliveira90/audio-rec.git C:\Code\audio-rec
cd C:\Code\audio-rec
git switch fix/device-invalidation-recovery
```

## 3. Load the build tools

Run this once in each new PowerShell window before any `cargo` command:

```powershell
.\scripts\with-msvc.ps1
```

It prints `MSVC linker: ...\VC\Tools\MSVC\...\link.exe`. If PowerShell blocks
the script, run `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` once.

## 4. Let Claude compile, test and fix it

Start `claude` in `C:\Code\audio-rec` and paste the prompt at the end of this
file. It compiles the code, fixes errors, runs the tests, and walks you
through a live test where you switch audio devices during a recording.

## 5. Move the program to the notebook

On the PC, after Claude reports `DONE`:

```powershell
.\scripts\with-msvc.ps1 cargo build --release
```

Copy `C:\Code\audio-rec\target\release\audio-rec.exe` to the notebook (USB
stick, OneDrive or a network share).

On the notebook, keep the old program and put the new one in its place:

```powershell
Copy-Item "$env:USERPROFILE\.cargo\bin\audio-rec.exe" "$env:USERPROFILE\.cargo\bin\audio-rec.old.exe"
Copy-Item "<where you copied it>\audio-rec.exe" "$env:USERPROFILE\.cargo\bin\audio-rec.exe"
```

The notebook needs no Rust or build tools to run it.

## 6. Test on the notebook

The device drop happened on the notebook, so test there too:

1. Start a recording: `audio-rec record -o test-device-loss.wav`
2. Play some audio (a video) and talk for 30 seconds.
3. Drop the audio device. In PowerShell **as Administrator**, run
   `Restart-Service -Name Audiosrv -Force` (the sound cuts out for a few
   seconds). Or, if your headset was the default device before step 1, turn
   it off for 15 seconds and back on. Changing the default device in Windows
   settings does **not** trigger the bug: Windows keeps the old stream alive.
   Wait 30 seconds, still playing and talking.
4. Stop the recording: `audio-rec stop` in a second window.

It worked when:

- the terminal shows `device lost - reconnecting...`, then
  `device reconnected, recording resumed`;
- if the video goes quiet after the drop, pause and play it again: the
  player lost its stream too, which is Windows behaviour, not the recorder;
- the log shows `recording resumed`;
- `ffprobe test-device-loss.wav` shows a length close to how long you
  recorded, with sound after the device switch.

To go back to the old program: copy `audio-rec.old.exe` over `audio-rec.exe`.

## 7. Merge

When both tests pass, open a pull request from
`fix/device-invalidation-recovery` to `master` on GitHub and merge it.

## Prompt for Claude on the PC

```text
THE JOB
Compile, test and finish the device-loss recovery fix on branch `fix/device-invalidation-recovery`. My last session wrote it on a machine with no MSVC linker, so it has never been compiled; expect compile errors and fix them.

The bug: when the Windows audio endpoint is invalidated mid-recording (HRESULT 0x88890004, AUDCLNT_E_DEVICE_INVALIDATED), the capture loops used `?` on GetNextPacketSize/GetBuffer/ReleaseBuffer, so the capture threads returned and died. Nothing checked them until stop. A 35-minute recording kept 3 minutes and reported success.

The change, to verify and correct:
- `core/src/wasapi/capture_engine.rs:416` `classify_capture_fault`: DEVICE_INVALIDATED and SERVICE_NOT_RUNNING are recoverable, everything else fatal. `CaptureHealth` (line 429) tracks device loss and recovery count. `fill_wallclock_silence` (line 474) keeps the WAV length equal to wall-clock time.
- `core/src/wasapi/loopback.rs:581` `handle_loopback_fault` and `core/src/wasapi/microphone.rs:572` `handle_microphone_fault`: on a recoverable fault, rebuild the client with `activate_*_client` at most once per second and keep writing silence; a replacement device with a different sample rate or channel count is not adopted.
- `core/src/commands/record.rs:775` onward: `audio_state` gains `loopback_device_lost`, `mic_device_lost`, `device_recoveries`, plus `warning` events with code `device_lost` and `device_recovered`.
`core/src/wasapi/process_loopback.rs` already handled this error and was left unchanged.

Then guide me through a live test on this PC: I start a recording, play audio and talk, switch the default playback device or reconnect my headset, and stop. Tell me each step and what to look for, and read the output I paste back.

THE WHY
I'm fixing audio-rec, my meeting recorder, so a dropped audio device no longer loses the rest of a recording. I'll copy the built `audio-rec.exe` to my notebook, which can't build Rust, and test it there too.

WHERE THINGS STAND
As of 2026-11-13, `EricOliveira90/audio-rec`, branch `fix/device-invalidation-recovery` at the commit after `bbe2a66` (the one adding this file), cloned at `C:\Code\audio-rec`. Where live state disagrees with this snapshot, trust live state; my Settled decisions still stand.

* Read first, in order: `plans/device-invalidation-recovery-pc.md` (this guide), `CLAUDE.md`, then the four files named in THE JOB.
* Settled: recover and keep recording rather than stop the session; fill silence during the outage so system audio and microphone stay aligned.
* Settled: commit `5a7ccf8` also carries earlier work that was never compiled either: `core/src/disk_space.rs`, capture-clock timing for offset and drift correction in `capture_engine.rs` and `recorder.rs`, and `core/tests/audio_sync.rs`. Make it compile too; plans/audio-sync-offset-drift-implementation.md describes it.
* Watch for: `cargo` needs the MSVC linker. Run `.\scripts\with-msvc.ps1` once per PowerShell window first; in Git Bash, `/usr/bin/link` shadows MSVC's linker, so run cargo from PowerShell.
* Watch for: `core/tests/audio_sync.rs` calls `ffmpeg`, so FFmpeg must be on PATH.
* Watch for: the repo is public. Keep personal names and meeting titles out of code, commits and files.

THE GUARDRAILS

* Only touch the files this branch changed, plus fixes the compiler or tests require. Leave everything else alone.
* Follow `CLAUDE.md`'s antivirus rules: no process enumeration, no hooks, no hidden processes beyond the documented FFmpeg flag.
* Make routine judgment calls yourself. If my premise looks wrong, say so in a sentence and carry on. Stop and ask me only when a fix would change recording behaviour beyond device-loss recovery, or before you push, merge or delete anything; first finish whatever doesn't depend on my answer.

DONE MEANS

* `cargo build --workspace` and `cargo test --workspace` pass in PowerShell after `.\scripts\with-msvc.ps1`; quote the test summary lines.
* The live device-switch test on this PC shows a `device_lost` warning, then `device_recovered`, and the saved WAV's `ffprobe` length matches the recording time with audio after the switch. If the test fails, the cause is named and fixed, and the test passes on a rerun.
* `cargo build --release` has produced `target\release\audio-rec.exe`.
* Fixes are committed on `fix/device-invalidation-recovery`, with the cause of each in its message. Do not push until I say so.
* Keep the deliverable to the smallest changes that make the build, tests and live test pass.
* When you finish, tell me the commit SHAs and the path of the release exe, then three short parts. Found: what you did or found. Scope: `DONE` when every DONE MEANS bullet above is met; otherwise `NOT DONE`, naming each unmet bullet, why, and what would unblock it. Follow-ups: each tagged `REQUIRED` (this job's outcome stays unreached without it; say what, and who acts), `CLEANUP` (residue you left and didn't clear, with the cost of leaving it), or `IDEA` (new scope: file it as a GitHub issue labelled `proposed` and give only the link); or `none`. Nothing more.
```
