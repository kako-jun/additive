# ADDITIVE-13 - 画像トランジション「添加物」シリーズ

2枚の画像を繋ぐトランジション演出を `(from, to, t) -> RGBAフレーム` という統一契約に
乗せた添加物群。name-name の場面転換に流しつつ、orber 流に CLI + web の単体製品としても出す。
フラグ機 **No.13 = 玉化ディゾルブ**（パトレイバー 廃棄物13号＋食品添加物 E番号 のもじり）。

## ビルド・テスト

```bash
cargo build                                   # CLI は gpu feature 有効（wgpu）
cargo test                                    # feature 無し = CPU リファレンスのみ（速い）
cargo test -p additive-core --features gpu    # GPU パリティテスト（Vulkan adapter が要る）
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
# wasm ターゲットも通すこと（core / wasm が wasm32 でビルド可能であること。gpu は付けない）
cargo build --target wasm32-unknown-unknown -p additive-core -p additive-wasm
```

レンダラ選択は `additive --renderer cpu|gpu`（デフォルト gpu、adapter 取得失敗時は cpu フォールバック）。

## ドキュメント

| ファイル | 内容 | 言語 |
|---|---|---|
| `README.md` | エンドユーザー向けの使い方 | 英語（マスター） |
| `docs/overview.md` | 設計思想・トランジション契約・レンダラ選定 | 英語 |
| `docs/roadmap.md` | 完了済み・残タスク（内部運用メモ） | 日本語 |
| `CLAUDE.md` | AI 向け内部ドキュメント | 日本語 |

README は英語マスター、overview は英語、roadmap と CLAUDE は日本語（内部用）。orber と同じ規約。

## ソース構成

orber と同じ Cargo workspace 構成。純粋描画コアだけを GUI / WASM から依存できるよう、
I/O と子プロセスは CLI 側に隔離する。

```
additive/
├── Cargo.toml              # [workspace] members = ["crates/core", "crates/cli", "crates/wasm"]
├── .cargo/config.toml      # wasm32 用 getrandom_backend cfg（#2 の orber-core 依存に備える）
└── crates/
    ├── core/               # additive-core: 純粋トランジションコア（wasm ビルド可・I/O 無し）
    │   └── src/
    │       ├── lib.rs              # timeline() 等
    │       ├── transition.rs       # trait Transition + by_name / all レジストリ
    │       └── transitions/
    │           ├── mod.rs
    │           └── crossfade.rs    # No.0 リファレンス（CPU、パリティオラクル）
    ├── cli/                # additive: CLI バイナリ（image::open / 連番PNG / 将来 ffmpeg）
    └── wasm/               # additive-wasm: ブラウザ向け wasm-bindgen ラッパー
```

## 主要な設計判断

- **レンダラは wgpu（Rust + WGSL）の1本に統一する**。CLI(native ヘッドレス) と
  ブラウザ(WebGPU / WebGL2 フォールバック) が**同じ WGSL** を走らせ、出力が一致する。
  理由は2要件の同時成立: ①全添加物でブラウザ・リアルタイムプレビューが要る、
  ②CLI と web の生成結果が同じになる。CPU-wasm 1本は①を満たせず、CPU-CLI + WebGL は
  ②を満たせない（別レンダラ＝別ピクセル）。wgpu だけが両立する。詳細は `docs/overview.md`
- **raw WebGL / raw WebGPU-in-JS は採らない**。シェーダが Rust の外に出ると orber と同じ
  2コードベース分裂が再発する。wgpu はシェーダ(WGSL)を Rust の内側に置けるのが本質
- **`additive-core` の CPU レンダラはリファレンス（パリティオラクル）**。プロダクション
  ではない。wgpu 出力をこれと突き合わせて検証する。web GUI には wgpu 着地まで CPU で
  暫定プレビューを出させる
- **トランジション契約は `(from, to, t) -> RgbaImage`**。`from` と `to` は同寸法前提
  （呼び出し側でリサイズ）。`t` は 0.0..=1.0 にクランプ
- **No.13 玉化ディゾルブは `orber-core` を依存して玉を借りる**（cluster→orb、再発明ゼロ）。
  orber-core は crates.io 未公開なので **git 依存**で引く（#2）。orber 本体に `--transition`
  は生やさない＝orber の「1入力スタイライザ」identity を濁さない
- **透過（アルファ）出力はブラウザ側で PNG-in-MOV muxer を再利用する**。orber が
  ffmpeg.wasm の vp9-alpha と散々戦った末に着地した JS-only QuickTime muxer
  （`orber/web/src/lib/movMuxer.ts`、PNG codec / rgba 32bit lossless / NLE 全対応）を
  流用し、同じ轍を踏まない。ベイク（不透明 mp4）は ffmpeg 子プロセスで先に通す
- **添加物の粒度 = だいたい orber 1個分の視覚世界**。パラメータ単位で別添加物にしない

## 関連プロジェクト

- [orber](https://github.com/kako-jun/orber) — 玉エンジンの供給元（`orber-core`）。No.13 が依存
- [name-name](https://github.com/kako-jun/name-name) — 消費側。場面転換に透過 webm/mov を流す

## 技術ルール

- コミットメッセージに Co-Authored-By を付けない
- Python 実行は `uv run python3`
