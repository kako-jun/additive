# ADDITIVE-13 roadmap（内部運用メモ）

## 確定した設計（2026-05-31, session549）

- 命名: **ADDITIVE-13**（additive＝添加物＋13号 designation）。フラグ機 No.13＝玉化ディゾルブ
- レンダラ: **wgpu（Rust + WGSL）1本**で native(CLI) + browser(WebGPU) を賄い、出力一致。
  2要件（①リアルタイムプレビュー ②CLI=web 同一）が wgpu 以外を全て脱落させた
- `additive-core` の CPU 経路はリファレンス／パリティオラクル（プロダクションではない）
- No.13 は `orber-core` を git 依存で借りて玉を出す。orber に `--transition` は生やさない
- アルファ出力は orber の PNG-in-MOV muxer（`movMuxer.ts`）流用。ffmpeg.wasm vp9-alpha は踏まない
- **添加物は2種類（#23, session581）**: 共有 `Additive` 識別子（designation/name/description）の下に
  `Transition: Additive`（`(from,to,t)->frame`）と `Generator: Additive`（`render(w,h,t,inputs)`＝0/1入力の
  素材生成、#19/#20/#21 用）。`all()`/`by_name()` は `enum AdditiveItem` の union を返す。`shader_wgsl()` は
  両契約とも `Option`。ジェネレータ実体は未実装＝型だけ先に確定（量産#8・配線#5 の前に抽象を決めた）

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
- [~] **#2 No.13 玉化ディゾルブ** — **実装完了（PR、session550）**: orber-core を git 依存（core の gpu feature 内に optional dep で隔離）で追加し `extract_clusters`+`drop_dominant` で玉プール抽出。`orb_dissolve.{rs,wgsl}` 新設。to 背景 ＋ from フェードアウト ＋ orber コンベア（[-r,1+r] wrap・per-orb 1x/2x/3x・breathing）で玉が左→右に漂い、sine 包絡で t=0 出現なし→中盤ピーク→t=1 消滅。GPU 経路（`GpuRenderer::render_orbs`、binding4=orb 配列 uniform）を本線、CPU は orber `render_static`(tiny-skia) でリファレンス。CPU/GPU 厳密パリティは課さず機構レベルで検証。No.0 crossfade のパリティ6件は維持。実 Intel GPU(ADL-N) で動作確認、`/tmp/orb13/` 12フレーム目視で from→玉→to を確認。**残: 視覚品質の数値調整（kako-jun）。方向/速度/count/orb-size のパラメータ化と full-occlusion 再定義は #14 で完了**
- [x] **#14 No.13 full-occlusion 再定義 + パラメータ化** — **完了（session）**: No.13 を「半透明クロスフェード」から「玉の幕で全面被覆 → 下で base を hard-swap → 幕が引いて to」の wipe に再定義。`base_from_weight`（t=0.5 で ±0.05 micro-cf の hard swap）+ `occlusion_envelope`（[0.40,0.60] プラトー）で半径と不透明度を駆動。玉は count から near-square グリッド（既定 8×8、`COVER_OVERLAP=1.85`・opaque core 0.7r）で gap-free 配置、コンベアは全玉共有の単一オフセット（1セル内 wrap でグリッド間隔維持）+ **toroidal 距離**（CPU/WGSL 両方）で wrap 継ぎ目を消し full occlusion を保証。`MAX_ORBS` 16→128、`OrbConfig{count,speed,direction,orb_size}` 追加、CLI に `--count/--speed/--direction(lr/rl/tb/bt)/--orb-size`（orb-dissolve 専用、crossfade に無害）配線。WGSL/gpu.rs を新契約に同期（`OrbParams{t,orb_count,aspect_x,aspect_y}`、aspect 補正距離・hard swap・0.7r core を CPU と一致）。**占有テスト: t=0.5 で base=from と base=to の出力差が CPU/GPU とも mean≈0.0000（CPU は max=0）= 玉が下を完全に隠す。full occlusion は t∈[0.0,0.65] に渡る**。端点 t=0≈from / t=1≈to、No.0 crossfade パリティ6件維持、wasm32 ビルド維持。実 Intel GPU(ADL-N) で `/tmp/occ_*.png` 目視（t=0.5 で from/to 透けず玉で充填、`--count/--orb-size/--direction/--speed` 各々が出力を変える数値確認 21万px 差）
- [~] **#3 動画エンコード** — **ベイク完了（PR、session）**: `crates/cli/src/video.rs` 新設（I/O・子プロセスは CLI 側に隔離、core は純粋のまま）。`--output` の拡張子で種別推論（`.png`=単一 peek、`.mp4`=H.264/yuv420p、`.webm`=VP9/yuv420p）。`--duration-ms`(既定 2500) / `--fps`(既定 30) 追加、frame_count = duration_ms*fps/1000（最低2、saturating）。連番 PNG を tempdir に書き `ffmpeg -framerate {fps} -i frame_%05d.png` で結合。ffmpeg 不在は install 案内付きエラー、非ゼロ終了は stderr 付き。レンダラは既存 `--renderer cpu|gpu` を尊重。**実機検証: 320x240→実 256x256 で orb-dissolve mp4(h264/60f) + crossfade webm(vp9/36f) を生成、ffprobe で codec/解像度/フレーム数確認、先頭/末尾フレーム差分も確認**。既存 13 テスト維持、wasm32 ビルド維持。**残（#3 follow-up）: アルファ .mov** — to 背景を描かず from+演出を straight-alpha で出すオーバーレイ経路が WGSL+CPU 両方に要る（core 改修＋パリティ）ため別途。CLI は `--alpha`/`.mov` を予約し未実装エラーで明示拒否（無言で不透明ベイクしない）
- [x] **#13 GpuRenderer リソースキャッシュ** — **完了（session）**: `render`/`render_orbs` がフレーム毎に shader module・render pipeline・bind group layout・texture・readback buffer を再生成していた問題を解消。`GpuRenderer` に4つのキャッシュを追加し、`RefCell<HashMap>` で内側可変に。pipeline/bind-group-layout/sampler は **shader 文字列ごと**（crossfade 用・orb 用で別レイアウトなので別キャッシュ）、from/to/target テクスチャと padded readback buffer は **`(width,height)` ごと**にキャッシュ。フレーム毎に作り直すのは bind group と小さな uniform だけ。`FrameRenderer::Gpu` は肥大化したので `Box` 化。**キャッシュ実効テスト追加**（`caches_resources_across_a_clip`）: 16フレームのクリップ後、4キャッシュとも entry=1（=shader 1回コンパイル・サイズ1回確保）、2つ目のサイズ追加で sized のみ +1（pipeline は据え置き）を実 GPU で検証。No.0 パリティ・No.13 機構テスト含め GPU 全21テスト維持、feature 無し / wasm32 ビルド維持、clippy -D warnings / fmt 通過。**FPS 計測（640x360, GTX 1050 Ti, 300フレーム連番PNG、I/O 込み）: crossfade 166.9→222.3 fps（+33%）、orb-dissolve 28.4→30.5 fps（+7%、orb 配列 uniform アップロードと fragment コストが支配的なため pipeline キャッシュの寄与は相対的に小さい）**
- [ ] **#4 wasm プレビュー + WebGPU** — ブラウザで wgpu リアルタイム、書き出しは WebCodecs / MOV muxer
- [ ] **#5 Web GUI** — Astro + Solid + Tailwind、orber 流、CLI と同一出力。DESIGN.md を実装
- [ ] **#6 サブドメイン公開** — `additive.llll-ll.com`（CF Pages）
- [ ] **#7 name-name 配線** — 透過 webm/mov を事前レンダして場面転換に流す
- [ ] **#8 添加物拡充（umbrella）** — No.14+ にじみ / 光漏れ / グリッチ …

## 完了（doctrine, session581）

- [x] **#23 ジェネレータ抽象（規律4）** — `Transition` 固定契約 → `Additive` supertrait + `Transition`/`Generator`
  + `enum AdditiveItem` union。3択（marker拡張/別trait/製品分割）から Option 2 採用。`shader_wgsl()` Option 化、
  No.0/No.13 移行（parity 実機 byte 一致）、CLI/wasm 配線、Generator 契約 proof テスト2件（core 26→28）。
  新ジェネレータ効果（#19/#20/#21）は別 Issue のまま＝型だけ先に作った。docs/overview.md・CLAUDE.md・本ファイル更新。
- [x] **#24 pipeline ビルダー単一化（規律2/OCP）** — `build_crossfade_pipeline`/`build_orb_pipeline` →
  単一 `build_pipeline(shader, BindShape)` + `(shader, BindShape)` キーの1キャッシュ。差を `enum BindShape { Crossfade, Orb }`
  に閉じ込め、新効果は variant 追加だけ。`sampler_entry()` 抽出。render/render_orbs と sized cache は据え置き。
  実機（Apple A18 Pro）で出力 byte 一致＝パリティ非破壊。gpu.rs 正味 -17 行。
- 残 doctrine finding（低優先・該当 Issue 着手時）: F2 `orb_instances` の grid を pure helper に（#8 時）/ F3 CPU/WGSL parity helper 拡大 / 無限cache eviction（#5 Web GUI 時）。

## 参照

- orber CHANGELOG #184 / #192: 透過動画は ffmpeg.wasm vp9-alpha が wasm 単スレッドのバグ・
  メモリ枯渇で破綻 → PNG-in-MOV（QuickTime atom 直書き、`movMuxer.ts` ~280行）に着地。
  ADDITIVE-13 のアルファ出力はこれを流用する
