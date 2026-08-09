# AppKit 基礎メモリ Baseline

Issue #60 の Task 1–2 用の実測記録。`scripts/agent/measure-appkit-memory.sh` が上書き生成する。

## 測定条件

- Date: 2026-08-09T06:04:49Z
- Commit: `e3a94609cd99dcf044ae4e91dd5a82bb77d62a0d`
- macOS: 26.5.2
- Architecture: arm64
- CPU: Apple M1
- Physical memory: 17179869184 bytes
- Build: `cargo build --release`, `render-stats` enabled
- Runs: 5 per case, separate process, fixed 800x600 window, no interaction, 5000ms stabilization

## ケース

| Case | Configuration |
|---|---|
| A | NSApplication + NSWindow + empty NSView |
| B | NSApplication + NSWindow + empty TreeHostView |
| C | B + TreeHostView.wantsLayer = true |
| D | graphics-demo initial Fills tab |

## 全サンプル

| Case | Run | Physical Footprint (MiB) | RSS (MiB) | TreeHosts attached/hidden | NSViews | Layer-backed NSViews | CALayers |
|---|---:|---:|---:|---:|---:|---:|---:|
| A | 1 | 13.92 | 69.86 | 0/0 | 1 | 1 | 1 |
| B | 1 | 13.64 | 69.61 | 1/0 | 1 | 1 | 1 |
| C | 1 | 13.53 | 69.58 | 1/0 | 1 | 1 | 1 |
| D | 1 | 17.85 | 75.09 | 8/6 | 63 | 63 | 94 |
| A | 2 | 13.81 | 69.75 | 0/0 | 1 | 1 | 1 |
| B | 2 | 13.56 | 69.55 | 1/0 | 1 | 1 | 1 |
| C | 2 | 13.63 | 69.59 | 1/0 | 1 | 1 | 1 |
| D | 2 | 17.86 | 75.11 | 8/6 | 63 | 63 | 94 |
| A | 3 | 13.80 | 69.75 | 0/0 | 1 | 1 | 1 |
| B | 3 | 13.63 | 69.59 | 1/0 | 1 | 1 | 1 |
| C | 3 | 13.58 | 69.56 | 1/0 | 1 | 1 | 1 |
| D | 3 | 17.99 | 75.22 | 8/6 | 63 | 63 | 94 |
| A | 4 | 13.66 | 69.59 | 0/0 | 1 | 1 | 1 |
| B | 4 | 13.63 | 69.61 | 1/0 | 1 | 1 | 1 |
| C | 4 | 14.11 | 70.08 | 1/0 | 1 | 1 | 1 |
| D | 4 | 17.85 | 75.09 | 8/6 | 63 | 63 | 94 |
| A | 5 | 13.89 | 69.83 | 0/0 | 1 | 1 | 1 |
| B | 5 | 13.64 | 69.61 | 1/0 | 1 | 1 | 1 |
| C | 5 | 13.61 | 69.58 | 1/0 | 1 | 1 | 1 |
| D | 5 | 17.85 | 75.09 | 8/6 | 63 | 63 | 94 |

## 中央値と差分

| Case | Physical Footprint median (MiB) | range (bytes) | RSS median (MiB) |
|---|---:|---:|---:|
| A | 13.81 | 14321536–14600064 | 69.75 |
| B | 13.63 | 14223168–14305152 | 69.61 |
| C | 13.61 | 14190464–14796672 | 69.58 |
| D | 17.85 | 18712448–18859968 | 75.09 |

- B - A: -0.19 MiB
- C - B: -0.02 MiB
- D - C: 4.23 MiB

## Raw JSON

```json
{"case":"A","run":1,"snapshot":{"physical_footprint_bytes":14600064,"resident_bytes":73252864,"attached_tree_host_count":0,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"B","run":1,"snapshot":{"physical_footprint_bytes":14305152,"resident_bytes":72990720,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"C","run":1,"snapshot":{"physical_footprint_bytes":14190464,"resident_bytes":72957952,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"D","run":1,"snapshot":{"physical_footprint_bytes":18712448,"resident_bytes":78741504,"attached_tree_host_count":8,"hidden_tree_host_count":6,"native_nsview_count":63,"layer_backed_nsview_count":63,"live_calayer_count":94,"live_shape_layer_count":4,"live_text_layer_count":2,"live_gradient_layer_count":2,"live_mask_layer_count":4,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":18,"text_layers_created":4}}
{"case":"A","run":2,"snapshot":{"physical_footprint_bytes":14485376,"resident_bytes":73138176,"attached_tree_host_count":0,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"B","run":2,"snapshot":{"physical_footprint_bytes":14223168,"resident_bytes":72925184,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"C","run":2,"snapshot":{"physical_footprint_bytes":14288768,"resident_bytes":72974336,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"D","run":2,"snapshot":{"physical_footprint_bytes":18728832,"resident_bytes":78757888,"attached_tree_host_count":8,"hidden_tree_host_count":6,"native_nsview_count":63,"layer_backed_nsview_count":63,"live_calayer_count":94,"live_shape_layer_count":4,"live_text_layer_count":2,"live_gradient_layer_count":2,"live_mask_layer_count":4,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":18,"text_layers_created":4}}
{"case":"A","run":3,"snapshot":{"physical_footprint_bytes":14468928,"resident_bytes":73138176,"attached_tree_host_count":0,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"B","run":3,"snapshot":{"physical_footprint_bytes":14288768,"resident_bytes":72974336,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"C","run":3,"snapshot":{"physical_footprint_bytes":14239552,"resident_bytes":72941568,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"D","run":3,"snapshot":{"physical_footprint_bytes":18859968,"resident_bytes":78872576,"attached_tree_host_count":8,"hidden_tree_host_count":6,"native_nsview_count":63,"layer_backed_nsview_count":63,"live_calayer_count":94,"live_shape_layer_count":4,"live_text_layer_count":2,"live_gradient_layer_count":2,"live_mask_layer_count":4,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":18,"text_layers_created":4}}
{"case":"A","run":4,"snapshot":{"physical_footprint_bytes":14321536,"resident_bytes":72974336,"attached_tree_host_count":0,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"B","run":4,"snapshot":{"physical_footprint_bytes":14288704,"resident_bytes":72990720,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"C","run":4,"snapshot":{"physical_footprint_bytes":14796672,"resident_bytes":73482240,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"D","run":4,"snapshot":{"physical_footprint_bytes":18712448,"resident_bytes":78741504,"attached_tree_host_count":8,"hidden_tree_host_count":6,"native_nsview_count":63,"layer_backed_nsview_count":63,"live_calayer_count":94,"live_shape_layer_count":4,"live_text_layer_count":2,"live_gradient_layer_count":2,"live_mask_layer_count":4,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":18,"text_layers_created":4}}
{"case":"A","run":5,"snapshot":{"physical_footprint_bytes":14567296,"resident_bytes":73220096,"attached_tree_host_count":0,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"B","run":5,"snapshot":{"physical_footprint_bytes":14305152,"resident_bytes":72990720,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"C","run":5,"snapshot":{"physical_footprint_bytes":14272384,"resident_bytes":72957952,"attached_tree_host_count":1,"hidden_tree_host_count":0,"native_nsview_count":1,"layer_backed_nsview_count":1,"live_calayer_count":1,"live_shape_layer_count":0,"live_text_layer_count":0,"live_gradient_layer_count":0,"live_mask_layer_count":0,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":0,"text_layers_created":0}}
{"case":"D","run":5,"snapshot":{"physical_footprint_bytes":18712448,"resident_bytes":78741504,"attached_tree_host_count":8,"hidden_tree_host_count":6,"native_nsview_count":63,"layer_backed_nsview_count":63,"live_calayer_count":94,"live_shape_layer_count":4,"live_text_layer_count":2,"live_gradient_layer_count":2,"live_mask_layer_count":4,"image_cache_bytes":0,"vector_raster_cache_bytes":0,"layers_created":18,"text_layers_created":4}}
```

## 観察と次の判断

この Issue は計測基盤と baseline のみを提供する。次の最適化は C-B の実測差とばらつきをレビューしてから別 Issue で決定する。
