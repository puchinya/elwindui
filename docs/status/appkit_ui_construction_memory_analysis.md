# AppKit UI構築段階メモリ分析

Issue #60 の次段階調査記録。ここでは最適化を実装せず、`graphics-demo`が誘発するsystem framework/runtime allocationを、UI構築段階ごとに分離した。

## 測定方法

- Date: 2026-08-09
- Environment: macOS 26.5.2 / Apple M1 / 16 GiB
- Build: `render-stats`を有効にしたrelease build
- Standard measurement: 各caseを別プロセスで5回起動し、表示後5,000 msにAppKit diagnostics JSONと同一PIDの`vmmap -summary`を採取した。
- Raw samples: [`appkit_ui_construction_memory.md`](appkit_ui_construction_memory.md) の全sample・中央値・範囲。生のstdoutと`vmmap`は `.agent-state/issues/60/ui-construction-20260809T070615Z/` にのみ保存した。
- MallocStackLogging: G/H/I/Jを別プロセスで`MallocStackLogging=full`により1回ずつ採取し、`malloc_history -allBySize`を解析した。生ログは `.agent-state/issues/60/ui-construction-msl-20260809T071200Z/` にのみ保存した。

MallocStackLoggingは常駐メモリを増やし、起動後にloggingを開始する。そのためMSLの絶対値や通常測定との差を混ぜず、**同一MSL条件のstage差におけるallocation stackの帰属だけ**に使う。`FRAG SIZE`はallocatorが保持するcapacityであり、live allocationや特定objectの所有者へは帰属させない。

## Case比較

| Case | 内容 | Footprint median | MALLOC allocated | FRAG | CoreAnimation dirty | NSView | TreeHost | CALayer |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| A | 空`NSView` | 14.24 MiB | 7,950 KiB | 9,650 KiB | 64 KiB | 1 | 0 | 1 |
| E | 空`TreeHostView` | 13.85 MiB | 7,891 KiB | 9,165 KiB | 64 KiB | 1 | 1 | 1 |
| F | `TabView`、0 tab | 15.72 MiB | 10,338 KiB | 9,146 KiB | 96 KiB | 7 | 1 | 11 |
| G | 空contentの1 tab | 16.06 MiB | 11,011 KiB | 9,327 KiB | 128 KiB | 15 | 2 | 22 |
| H | 空contentの7 tab | 18.13 MiB | 14,083 KiB | 9,775 KiB | 336 KiB | 63 | 8 | 82 |
| I | Jと同じ7-tab UI/state、paint無効 | 18.17 MiB | 14,069 KiB | 9,819 KiB | 336 KiB | 63 | 8 | 82 |
| J | 通常graphics-demo、Fills paint有効 | 18.72 MiB | 14,300 KiB | 10,011 KiB | 432 KiB | 63 | 8 | 94 |

| Transition | Footprint | MALLOC allocated | FRAG | CoreAnimation | NSView | TreeHost | CALayer |
|---|---:|---:|---:|---:|---:|---:|---:|
| E - A | -0.39 MiB | -59 KiB | -485 KiB | 0 KiB | 0 | +1 | 0 |
| F - E | **+1.88 MiB** | **+2,447 KiB** | -19 KiB | +32 KiB | +6 | 0 | +10 |
| G - F | +0.34 MiB | +673 KiB | +181 KiB | +32 KiB | +8 | +1 | +11 |
| H - G | **+2.06 MiB** | **+3,072 KiB** | **+448 KiB** | **+208 KiB** | **+48** | **+6** | **+60** |
| I - H | +0.05 MiB | -14 KiB | +44 KiB | 0 KiB | 0 | 0 | 0 |
| J - I | +0.55 MiB | +231 KiB | +192 KiB | +96 KiB | 0 | 0 | +12 |

`untagged VM_ALLOCATE dirty`の中央値はAからJまで32 KiBで変化せず、今回のstage差の主因ではない。RSSの中央値と全metricの範囲は、raw measurement documentに記録している。特にA/F/G/I/Jには起動時framework初期化のばらつきがあり、単発値ではなく上表の中央値で比較する。

## Tabコスト

0 / 1 / 7 tabの比較はF / G / Hで行った。1本目のtabはF→GでFootprint +0.34 MiB、MALLOC allocated +673 KiB、FRAG +181 KiB、NSView +8、TreeHost +1、CALayer +11だった。

1 tabから7 tabへの6本追加（G→H）は、1本あたり平均でFootprint **+0.34 MiB**、MALLOC allocated **+512 KiB**、FRAG **+74.7 KiB**、NSView **+8**、TreeHost **+1**、CALayer **+10**である。これは線形性を仮定した値ではなく、同一の6本追加を割った参考値である。追加tabごとにpersistent content `TreeHostView`、tab chipのnative controls、backing layersが作られる実装と整合する。

## UI treeとrendererの分離

H→Iでは同数の7 tabを維持し、`GraphicsDemoCanvas`のpaint callbackだけを無効化した。中央値差はFootprint +0.05 MiB、MALLOC allocated -14 KiB、CALayer 0で、graphics-demoのUIElement/state型そのものはこの条件では大きなpersistent memoryを占めなかった。

I→JではFills描画を有効化した。Footprint **+0.55 MiB**のうち、MALLOC allocated +231 KiB、FRAG +192 KiB、CoreAnimation dirty +96 KiB、CALayer +12が観測された。従ってrendererは実測できるが、7 empty tab追加の+2.06 MiBやTabView shellの+1.88 MiBより小さい。PaintIslandやshape batchingを次の最優先候補にはしない。

## Allocation attribution

`malloc_history -allBySize`のlive malloc blockについて、末尾が`libsystem_malloc`であるstackのみを集計した。H-GではMSL条件のtracked live mallocが5,083.3 KiBから6,369.0 KiBへ+1,285.7 KiB、I-Jでは6,349.1 KiBから6,450.6 KiBへ+101.5 KiBだった。これはMSLの別条件値であり、通常測定の`vmmap` MALLOC ALLOCATED差と加算・比較しない。

### 直接allocation site

| Transition | direct allocation site | 増分Persistent Bytes |
|---|---|---:|
| H - G | `swift::swift_slowAllocTyped` | +303.3 KiB |
| H - G | `_objc_rootAllocWithZone` | +253.5 KiB |
| H - G | `_CFRuntimeCreateInstance` | +90.6 KiB |
| H - G | `AutoreleasePoolPage::autoreleaseFullPage` | +68.0 KiB |
| H - G | `AG::data::zone::alloc_persistent` | +59.6 KiB |
| H - G | `CoreAutoLayout::_table_addStorageBlock` | +52.9 KiB |
| H - G | `class_createInstance` | +46.3 KiB |
| H - G | `NSISSparseVectorAddTermWithPlaceValueCoefficientStartingIndex` | +38.1 KiB |
| I - J | `TFont::InitShapingGlyphs` | +16.0 KiB |
| I - J | `operator_new_impl` | +14.5 KiB |
| I - J | `_CFRuntimeCreateInstance` | +11.6 KiB |
| I - J | `cache_t::insert` | +10.5 KiB |
| I - J | `CGGlyphBuilderLockBitmaps` | +8.5 KiB |
| I - J | `CGGlyphBitmapCreate` | +8.4 KiB |
| I - J | `CA::Transaction::create` | +8.0 KiB |

H→Gの直接siteはSwift/Objective-C runtime、CoreFoundation、AutoLayout/AttributeGraphが主であり、ElwindUIの`Vec`/`HashMap`が直接数MiBを保持する結果ではない。I→Jはfont shaping、glyph bitmap、CoreAnimation transactionが主で、rendererの増分が小さいという通常測定と整合する。

### first project frame attribution

次表はdirect ownerではない。各stackをallocation側から呼出し元へ辿った最初の`graphics-demo`またはElwindUI frameで集計し、**system allocationを誘発したproject-side path**として表示する。AppKitのlazy initializationやrunloop非同期処理では、最初のproject frameが`application::run`となり、それ以上に細分化できないstackがある。

| Transition | first project frame | 増分Persistent Bytes |
|---|---|---:|
| H - G | `objc2::NSButton::buttonWithTitle` wrapper | +435.0 KiB |
| H - G | `InnerWindow::show` | +289.1 KiB |
| H - G | `application::run`（framework lazy/startup） | +253.3 KiB |
| H - G | symbol unavailable (`<deduplicated_symbol>`) | +162.0 KiB |
| H - G | `TreeHostView::replay_commands` | +94.4 KiB |
| H - G | `objc2::NSStackView::stackViewWithViews` wrapper | +44.8 KiB |
| H - G | `TreeHostView::new` | +15.3 KiB |
| H - G | `InnerTabView::insert_tab` | +6.9 KiB |
| I - J | `application::run`（framework lazy/runloop） | +79.3 KiB |
| I - J | `objc2::NSFont::fontWithDescriptor_size` wrapper | +9.6 KiB |
| I - J | `CATextLayer::new` wrapper | +6.4 KiB |
| I - J | ElwindUI core `RawVec` growth | +3.5 KiB |
| I - J | `CAShapeLayer::new` wrapper | +3.1 KiB |
| I - J | `render::paint::try_add_gradient_fill_layer` | +1.6 KiB |

このattributionは、tab追加でframework allocationを誘発する主な経路がnative tab chip / `NSButton` / `NSStackView` / window-show / host replayであることを示す。一方、renderer差には一部`try_add_gradient_fill_layer`が現れるが1.6 KiBに過ぎず、大きな割当は非同期AppKit/CoreText/CoreAnimation経路として`application::run`側に現れる。

## 結論と優先順位

1. graphics-demoの+4〜5 MiBのうち、最大の段階増加は**空tabを1本から7本へ増やすG→Hの+2.06 MiB**である。次点はTabView native shellを追加するE→Fの+1.88 MiB。
2. TabView/native AppKit objectは、F→HでFootprint約+2.40 MiB、MALLOC allocated約+3.75 MiBを伴う。FRAGは+629 KiBで、live allocationとは別枠である。
3. graphics-demoのUIElement/state型をempty contentから同型canvasへ替えるH→Iは+0.05 MiBであり、この条件では主要因ではない。
4. Fills rendererのI→Jは+0.55 MiB。CoreAnimation +96 KiB、CALayer +12を含むが、tab構築より小さい。
5. 次に最も価値が高い調査候補は、**inactive tabのnative chip/content hostの遅延生成または再利用**である。ただし既存のtab lifecycle・選択切替・resource解放の仕様に影響するため、このIssueでは実装しない。PaintIsland/shape batchingはこの計測だけからは低優先度とする。
