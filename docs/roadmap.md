# ADDITIVE-13 roadmap（内部運用メモ）

## 確定した設計（2026-05-31, session549）

- 命名: **ADDITIVE-13**（additive＝添加物＋13号 designation）。フラグ機 No.13＝玉化ディゾルブ
- レンダラ: **wgpu（Rust + WGSL）1本**で native(CLI) + browser(WebGPU) を賄い、出力一致。
  2要件（①リアルタイムプレビュー ②CLI=web 同一）が wgpu 以外を全て脱落させた
- `additive-core` の CPU 経路はリファレンス／パリティオラクル（プロダクションではない）
- No.13 は `orber-core` を git 依存で借りて玉を出す。orber に `--transition` は生やさない
- アルファ出力は orber の PNG-in-MOV muxer（`movMuxer.ts`）流用。ffmpeg.wasm vp9-alpha は踏まない

## 完了（scaffold, session549）

- [x] Cargo workspace（core / cli / wasm）+ orber ミラーのフォルダ構成
- [x] `trait Transition { designation / name / description / render_cpu }` + by_name/all レジストリ
- [x] No.0 crossfade（CPU リファレンス）実装 + テスト（端点純粋・中点平均）
- [x] CLI: `--from --to --transition --output --t / --frames --out-dir / --list`
- [x] wasm-bindgen ラッパー（`render_frame` / `transitions`）— CPU 暫定プレビュー
- [x] cargo build / test / clippy -D warnings / fmt / wasm32 ビルド 全通過
- [x] end-to-end 実証: 赤→青 crossfade 中点が正確な平均ピクセル（#763076）

## 残（Issue 化）

- [~] **#1 wgpu レンダラ基盤** — **native 部分 完了（PR、session549）**: `additive-core` の `gpu` feature に `GpuRenderer`(wgpu29) + `crossfade.wgsl`、CLI `--renderer cpu|gpu`(default gpu, cpu フォールバック)。パリティテストが実 Intel GPU(ADL-N) で CPU リファレンスと ±2/ch 一致を確認。rgba8unorm 非srgb + 256-byte 行アラインで一致担保。**残: ブラウザ/WebGPU 側は #4**
- [~] **#2 No.13 玉化ディゾルブ** — **実装完了（PR、session550）**: orber-core を git 依存（core の gpu feature 内に optional dep で隔離）で追加し `extract_clusters`+`drop_dominant` で玉プール抽出。`orb_dissolve.{rs,wgsl}` 新設。to 背景 ＋ from フェードアウト ＋ orber コンベア（[-r,1+r] wrap・per-orb 1x/2x/3x・breathing）で玉が左→右に漂い、sine 包絡で t=0 出現なし→中盤ピーク→t=1 消滅。GPU 経路（`GpuRenderer::render_orbs`、binding4=orb 配列 uniform）を本線、CPU は orber `render_static`(tiny-skia) でリファレンス。CPU/GPU 厳密パリティは課さず機構レベルで検証。No.0 crossfade のパリティ6件は維持。実 Intel GPU(ADL-N) で動作確認、`/tmp/orb13/` 12フレーム目視で from→玉→to を確認。**残: 視覚品質の数値調整（kako-jun）、方向/速度/k のパラメータ化**
- [ ] **#3 動画エンコード** — CLI: ffmpeg で不透明 mp4/webm（ベイク）／アルファは PNG-in-MOV
- [ ] **#4 wasm プレビュー + WebGPU** — ブラウザで wgpu リアルタイム、書き出しは WebCodecs / MOV muxer
- [ ] **#5 Web GUI** — Astro + Solid + Tailwind、orber 流、CLI と同一出力。DESIGN.md を実装
- [ ] **#6 サブドメイン公開** — `additive.llll-ll.com`（CF Pages）
- [ ] **#7 name-name 配線** — 透過 webm/mov を事前レンダして場面転換に流す
- [ ] **#8 添加物拡充（umbrella）** — No.14+ にじみ / 光漏れ / グリッチ …

## 参照

- orber CHANGELOG #184 / #192: 透過動画は ffmpeg.wasm vp9-alpha が wasm 単スレッドのバグ・
  メモリ枯渇で破綻 → PNG-in-MOV（QuickTime atom 直書き、`movMuxer.ts` ~280行）に着地。
  ADDITIVE-13 のアルファ出力はこれを流用する
