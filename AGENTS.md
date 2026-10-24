# AGENTS.md — telecomfort

Single-binary Rust desktop Telegram client (`eframe/egui` + `grammers` + `tokio` + `rusqlite`).
Docs: `docs/ARCHITECTURE.md`, `docs/DEVELOPER_GUIDE.md` (canonical 6-step feature path), `docs/USER_GUIDE.md`.

## Commands (always these exact forms)

```bash
cargo check --offline
cargo test --lib --offline    # ~130 tests (юниты + контракты + мост), ~5s
cargo clippy --offline        # must be 0 errors (lib + bin + android target)
cargo fmt                     # before every commit
```

Android: `./android/build-tgvoip.sh` (libtgvoip, one-time clone) then
`./android/build-rust.sh [debug|release]`; aarch64 check needs rustup
toolchain (`RUSTC=~/.rustup/.../bin/rustc`, NDK clang via `CC_<target>` env —
brew cargo can't see rustup targets). UI deps are
`cfg(not(target_os = "android"))`-gated; `BgEvent` lives in `src/events.rs`
so background code never touches `ui::`. New JNI op = arm in
`android_bridge::dispatch` + Kotlin call in `Core`/screen + host test.

## Offline cargo — hard constraint

- No network for cargo: **cannot add new crates**. Only versions already in
  `~/.cargo/registry` are usable (e.g. `cpal 0.18`, `pbkdf2 0.13`, `hmac 0.13`).
- Crypto versions must align or trait bounds fail: `sha2 0.11` + `hmac 0.13` +
  `pbkdf2 0.13` (match `grammers-crypto 0.11`'s own deps). `sha2 0.10` breaks this.

## Verify TL names in generated code, never from memory

grammers TL type/variant names are exact and surprising (`StarGift` not
`InvoiceStarGift`, `Messages::ChatFull::Full`, `StarsStatus::Status`,
`Updates::Combined`, `Peer::to InputPeer` vs `InputChannel::from(PeerRef)`).
Check before writing backend code:

```
target/debug/build/grammers-tl-types-*/out/generated_{types,functions,enums}.rs
```

## Architecture rules

- UI depends only on `telegram::TelegramService` (`Arc<dyn>`). Never call
  `GrammersBackend` methods from views except `connect_with_proxy` at login.
- UI thread never touches network: `rt.spawn` + `mpsc BgEvent` + `pump_events()`.
- New backend method = inherent `impl GrammersBackend` block + one-line
  delegation in the trait impl. **Trap**: inserting before
  `impl TelegramService for GrammersBackend` orphans the `#[async_trait]`
  attribute onto your inherent block — always re-attach it to the trait impl.
- Feature path: trait default (`bail!`) → backend → `MockTelegram` demo data →
  app method + pump arm → view → extend `mock_management_contract_15`.
  App starts in demo/mock mode, so UI is testable without keys.

## Library API quirks (verified)

- `cpal 0.18`: `SampleRate` is a `u32` alias; `build_input_stream` takes
  `StreamConfig` **by value**; clone it before moving into the match.
- `rodio 0.23`: `Decoder` yields `f32`; `sample_rate()/channels()` return
  `NonZero` (use `.get()`).
- `egui 0.37`: no `ImageButton` — use `Image::new(..).sense(Sense::click())`;
  scroll delta field is `smooth_scroll_delta`.
- `PeerRef.id` is a **field**, not a method. User `access_hash` comes from
  `GetPeerDialogs` (see `dialog_user`), never from the peer id alone.
- `upload_file` returns `Uploaded` with `.raw: InputFile` — use directly.

## Workflow

- Branch `feature/*` → commit → `gh pr create` → `gh pr merge --merge` → main.
  PRs merge cleanly (no required checks); verify with `cargo test` after merge.
- Commit messages in Russian, prefix `Паритет-N:` / topic. Keep tree clean.
