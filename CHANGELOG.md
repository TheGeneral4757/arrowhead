# Changelog

## [0.1.0](https://github.com/TheGeneral4757/arrowhead/releases/tag/v0.1.0) (2026-11-22)

First release, Linux only. arrowhead reads the four lanes of Funky Friday off your screen and presses each note on time, playing the Max preset. It comes as a desktop app (Live, Setup, Settings) and `arrowhead-cli`.

### Features

* Screen capture through xdg-desktop-portal ScreenCast over PipeWire, with clip recording and replay for testing.
* Detection that learns lane colours, holds, up/downscroll and which side you play from your screen, plus saved profiles.
* Note tracking, fitted scroll speed and presses at the predicted crossing, with a latency test and an adjustable offset.
* uinput virtual keyboard with a focus guard (KWin), stuck-key watchdog, release-all on exit or crash, panic hotkey and dry run.
* Global hotkeys through the GlobalShortcuts portal.
* Tauri 2 desktop app with Live, Setup and Settings pages, and a signed in-app updater for tarball installs.
