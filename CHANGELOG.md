# Changelog

## [0.2.0](https://github.com/TheGeneral4757/arrowhead/compare/v0.1.0...v0.2.0) (2026-11-25)


### Features

* **adapt:** fast-start readback, lane and class bias, per-song memory ([4f3c4ce](https://github.com/TheGeneral4757/arrowhead/commit/4f3c4ce26fbffe34cb55d4e27ed9a0ffb0193c94))
* **app:** History page with per-song hit-error histogram ([6ce5afa](https://github.com/TheGeneral4757/arrowhead/commit/6ce5afa043f2ecaa519d91bfb66241475e5ac4cf))
* **app:** Style page with presets, target, model preview and saved styles ([bd1b6bf](https://github.com/TheGeneral4757/arrowhead/commit/bd1b6bf08f3ea0c09ea72008fd733993a40f092d))
* **capture:** HUD side copies via AuxTap in every live backend ([0b1e815](https://github.com/TheGeneral4757/arrowhead/commit/0b1e8158d3ba10f0abdcd9c088bb34f88b0d1e70))
* **cli:** detect --profile replays a clip the way a live run starts ([c12238b](https://github.com/TheGeneral4757/arrowhead/commit/c12238b078aa3baa82f6bb5d36f4e0c2f7e6159d))
* **core:** AuxTap side copies of HUD areas outside the capture region ([91976c3](https://github.com/TheGeneral4757/arrowhead/commit/91976c3071b4c90b53fde70cb3231c536d9612e8))
* **core:** player model and accuracy controller (styles) ([6b8a9ee](https://github.com/TheGeneral4757/arrowhead/commit/6b8a9eef9b568bb8e91f6fe946019a548c46256a))
* **engine:** confirm ms readouts against Mean MS, judgment from the readout's colour ([f0d44bd](https://github.com/TheGeneral4757/arrowhead/commit/f0d44bded29e3257c787927724efcae2c9d88dff))
* **engine:** flight recorder saves the last 10 s of frames and presses when a note is missed ([9e2c609](https://github.com/TheGeneral4757/arrowhead/commit/9e2c609d4a6fdfde1b4d6423bd35f3f9395312be))
* **engine:** HUD thread with readback offset, accuracy feedback, history rows and live teaching ([e73037d](https://github.com/TheGeneral4757/arrowhead/commit/e73037d93c03e0d930860301bcc1d985e3612186))
* **engine:** OCR teacher logic: repaired reads, arithmetic-checked trust, element finding ([56994e0](https://github.com/TheGeneral4757/arrowhead/commit/56994e0bdb1ae207127d3bf549e427fe3b93eb3f))
* **engine:** OCR teaches the HUD reader and names songs; per-song memory applied at song start ([dea2f85](https://github.com/TheGeneral4757/arrowhead/commit/dea2f854ea25046568ff02163309e64824efc0ab))
* **engine:** per-lane and per-pattern corrections from readback, fast start per song, judgment by readout colour ([07b1899](https://github.com/TheGeneral4757/arrowhead/commit/07b189916ff65474cba3cbf35ff8aba9cd637ad2))
* **engine:** read both scores, detect your side, match my friend ([872dcb9](https://github.com/TheGeneral4757/arrowhead/commit/872dcb95788e37ff4da7533191b2cabcbc62611f))
* **history:** read the results screen with OCR and write its exact numbers into the history row ([cc270a9](https://github.com/TheGeneral4757/arrowhead/commit/cc270a91d826f6fe85ad75eb28b7a55f4370a09c))
* **hud:** HUD reader with taught glyph atlas, element parsers and readback matcher ([639185a](https://github.com/TheGeneral4757/arrowhead/commit/639185a063f263df4248f308607e4f16fe8a1734))
* **ocr:** ocrs-based OCR teacher crate, cli ocr, model packaging ([b760f22](https://github.com/TheGeneral4757/arrowhead/commit/b760f2233bd7f8c94b242b9e151de72303346c57))
* **overlay:** click-through overlay process and app spawn (v0.2) ([1719d9a](https://github.com/TheGeneral4757/arrowhead/commit/1719d9add9c9bcf0130ba6d652b9b316e0a7dfe7))
* **release:** Linux AppImage with in-app updates, user guide ([#14](https://github.com/TheGeneral4757/arrowhead/issues/14)) ([0f56ea1](https://github.com/TheGeneral4757/arrowhead/commit/0f56ea138c6b5a96112496307d3f5439fff381cc))
* **windows:** DXGI/WGC capture, SendInput, focus probe, hotkeys, MSI release job ([e461d45](https://github.com/TheGeneral4757/arrowhead/commit/e461d4501b40e7b3c8854135b2a3c7140bcbfa30))


### Bug Fixes

* **app:** overlay on the game's monitor and toggleable, no hang on stop, scores searched all song ([dbd6ec3](https://github.com/TheGeneral4757/arrowhead/commit/dbd6ec35223d45fd785fc7bd233e7b034af292f4))
* **app:** overlay setting in Settings, log why it is off, dev build includes the overlay ([69b6614](https://github.com/TheGeneral4757/arrowhead/commit/69b6614578ac5cf1aca59edbf51391d325580411))
* **app:** send keys on Windows instead of always playing dry ([4cb3ca5](https://github.com/TheGeneral4757/arrowhead/commit/4cb3ca5a47d4fd3a7afed1935005beece6ab484c))
* **core:** side copies skip rects a short capture buffer cannot hold ([971dce4](https://github.com/TheGeneral4757/arrowhead/commit/971dce483e0dcaed9346fb6840450570f3bc97fd))
* **core:** styled jacks keep note order in each lane ([996014d](https://github.com/TheGeneral4757/arrowhead/commit/996014d818fdeea54a8c47ac6d0cad62e548d462))
* **engine:** a key setup error no longer leaves a focus watcher running ([0965c14](https://github.com/TheGeneral4757/arrowhead/commit/0965c146cb25ffd24a1bf7c8e7951ca53de97e18))
* **engine:** end a song when the receptors are gone, not after a 3 s break ([a99ac84](https://github.com/TheGeneral4757/arrowhead/commit/a99ac84e18ed932e083286c599e14bfb70dd2625))
* **engine:** hit the first notes of a song started from the menu or countdown, and restart without relearning ([9348985](https://github.com/TheGeneral4757/arrowhead/commit/9348985b0ce480aa175df378cd80d3deceb2769a))
* **engine:** ignore repeated game frames when tracking notes, so holds and fast sections stay on time ([f3099ba](https://github.com/TheGeneral4757/arrowhead/commit/f3099bacc5aef495f1b863051bd62f1ab3b37129))
* **engine:** move the offset only on ms readouts the stats panel confirms ([0605299](https://github.com/TheGeneral4757/arrowhead/commit/06052996834ad73f3961dddf6cceb0f092ae7b5f))
* **engine:** read the stats panel's Mean MS with the right sign, undo font teaching that breaks the counts, save the offset only from songs that agree ([44f8e36](https://github.com/TheGeneral4757/arrowhead/commit/44f8e3656398778f676ebacf8a61fa160eff7ff6))
* **engine:** real-time offsets at any song rate, and lane corrections only after 20 samples each ([cbe5f72](https://github.com/TheGeneral4757/arrowhead/commit/cbe5f722d69ff6fc7f7771f1c2158880e439078c))
* **history:** one song memory per song even when OCR reads the title a little differently ([d3955b6](https://github.com/TheGeneral4757/arrowhead/commit/d3955b63cd488f3b060af681d96cadf488c51020))
* **history:** read results screens reliably: score and total checks, voting over several frames, 100% read as 100 ([8d8282b](https://github.com/TheGeneral4757/arrowhead/commit/8d8282bda9e2ec5fd49e34934950f323ce8a7f71))
* **hud:** read only our ms readout, teach the ms font from the first hit ([ffee808](https://github.com/TheGeneral4757/arrowhead/commit/ffee8087aedca60d2ba34744886dcd2dbdb8de8c))
* **overlay:** draw nothing over the lanes or the HUD during a run; Settings toggles only the overlay ([bc8f547](https://github.com/TheGeneral4757/arrowhead/commit/bc8f547f3562a11bdc51c6774bb02c1b34cb7003))
* **overlay:** pick the game's output by stream size, cap redraws at 30 Hz ([1635eb4](https://github.com/TheGeneral4757/arrowhead/commit/1635eb4d50a5e2b4442873417622d3b00471b449))
* **release:** ship the overlay in the Windows MSI and zip ([b5ea8c3](https://github.com/TheGeneral4757/arrowhead/commit/b5ea8c38242194fbfc8e37d1907635fa655786ca))

## [0.1.0](https://github.com/TheGeneral4757/arrowhead/releases/tag/v0.1.0) (2026-11-22)

First release, Linux only. arrowhead reads the four lanes of Funky Friday off your screen and presses each note on time, playing the Max preset. It comes as a desktop app (Live, Setup, Settings) and `arrowhead-cli`.

### Features

* Screen capture through xdg-desktop-portal ScreenCast over PipeWire, with clip recording and replay for testing.
* Detection that learns lane colours, holds, up/downscroll and which side you play from your screen, plus saved profiles.
* Note tracking, fitted scroll speed and presses at the predicted crossing, with a latency test and an adjustable offset.
* uinput virtual keyboard with a focus guard (KWin), stuck-key watchdog, release-all on exit or crash, panic hotkey and dry run.
* Global hotkeys through the GlobalShortcuts portal.
* Tauri 2 desktop app with Live, Setup and Settings pages, and a signed in-app updater for tarball installs.
