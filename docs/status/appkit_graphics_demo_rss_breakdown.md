# macOS graphics-demo RSS 内訳調査

Issue #60 の追加受入条件に対する、実プロセスの調査記録。目的はRSSを小さく見せることではなく、実際にmemory pressureとなる専有dirty memoryと、共有・clean resident memoryを区別することである。

## 採取条件と根拠

- Date: 2026-08-09
- macOS: 26.5.2 / Apple M1 / 16 GiB
- Build: `target/release/graphics-demo` と `target/release/examples/appkit-memory-baseline A`、`render-stats` 有効のrelease build
- graphics-demo: PID 83381、Fills初期タブを表示後に採取
- minimal: PID 83708、Case A（`NSApplication + NSWindow + empty NSView`）を表示後に採取
- Commands: `ps -o pid,rss,vsz,command -p <PID>`、`vmmap -summary <PID>`、`vmmap -wide <PID>`、`footprint -p <PID> -w`
- `vmmap -h` と `footprint -h` で、このOSでは上記オプションが利用可能なことを確認した。
- 生ログは調査用の `.agent-state/issues/60/` にのみ保存し、ここでは派生した集計値と結論を記録する。

この単発の同一セッション採取は差分の分類に使う。既存の5サイクルbaseline中央値（A: Footprint 13.81 MiB / RSS 69.75 MiB、D: 17.85 MiB / 75.09 MiB）を置き換えるものではない。

## まず数値の意味を分ける

| 指標 | graphics-demo | Case A | 差分 |
|---|---:|---:|---:|
| `ps` RSS | 83.30 MiB | 75.77 MiB | +7.53 MiB |
| `vmmap` Physical Footprint | 20.0 MiB | 15.4 MiB | +4.6 MiB |
| `footprint` dirty | 20 MiB | 15 MiB | +5 MiB（表示丸め） |
| `footprint` clean | 16 MiB | 13 MiB | +3 MiB |

`ps` のRSSはこのOSではresident set size（KiB単位）である。一方、`vmmap -summary` の全region resident合計はgraphics-demoで578.5 MiBになった。ここにはプロセスにmapされているdyld shared cache/共有frameworkのシステム全体residentページが含まれるため、RSSとして合計してはならない。

`footprint` のdirty合計は `vmmap` のPhysical Footprint 20.0 MiBと一致する。したがって、このプロセスに直接帰属するmemory pressureの主指標は約20 MiBであり、RSS 83.30 MiBではない。

RSS 83.30 MiBからFootprint 20 MiBを引いた約63 MiBはFootprintに含まれないresident memoryである。そのうち`footprint`が明示するcleanページは16 MiB（mapped file 15 MiB、`__TEXT` 1.23 MiB、`__LINKEDIT` 80 KiBなど）。残る約47 MiBは、`ps`にはresidentとして見える一方でこの出力だけではプロセス専有/共有を厳密に按分できないページである。`vmmap`のread-only librariesはsystem-wide residentとして440.4 MiBと表示されるため、この残差をshared frameworkの正確な容量として断定しない。少なくとも、この残差をElwindUIのprivate allocationとして扱う根拠はない。

## RSS/Footprintに関係するregion

`vmmap -summary` のgraphics-demo値を示す。resident/dirtyは同コマンドのregion値であり、`__TEXT`/`__LINKEDIT`のresidentは前述の理由でRSS内訳として足し上げない。

| Region | Virtual | vmmap resident | Dirty/private | graphics-demo - A | 削減可能性 |
|---|---:|---:|---:|---:|---|
| MALLOC zones | 45.7 MiB | 13.0 MiB | 13.0 MiB | 約+3.3 MiB dirty | A |
| MALLOC metadata | 1008 KiB | 624 KiB | 624 KiB | 0 | B |
| untagged `VM_ALLOCATE` | — | 864 KiB | 864 KiB | +416 KiB dirty | A（要帰属確認） |
| CoreAnimation | 688 KiB | 688 KiB | 688 KiB | +368 KiB dirty | B |
| IOSurface | 176 KiB | 128 KiB | 128 KiB | 0 | C |
| CoreGraphics | 48 KiB | 48 KiB | 48 KiB | 0 | C |
| stack | 9792 KiB | 208 KiB | 208 KiB | +80 KiB dirty | B |
| `__DATA` + `__DATA_DIRTY` | 42.1 MiB | 11.3 MiB | 2.8 MiB | 約+401 KiB dirty | B |
| `__DATA_CONST` | 34.5 MiB | 17.8 MiB | 176 KiB | +112 KiB dirty | C |
| mapped file | 241.5 MiB | 14.5 MiB | 0 | +3 MiB clean | C |
| `__TEXT` / `__LINKEDIT` | 1.8 GiB | 440.4 MiB | 0 | +384 KiB clean (`__TEXT`) | C |

`MALLOC_SMALL`の13.0 MiBはすべてprivate dirtyである。詳細mapではgraphics-demoとCase Aの差分は `DefaultMallocZone` が+2.9 MiB、`QuartzCore` zoneが+80 KiBだった。したがって、最大の削減候補は大きなCoreAnimation surfaceではなく、default allocatorに置かれたgraphics-demo固有の状態である。ただし`vmmap`だけでは、その2.9 MiBをElwindUIのRenderTree、デモ用データ、AppKitオブジェクトのどれに帰属させるかは確定できない。

## graphics-demo固有の増加

`footprint`の同一セッション差分はdirty約+5 MiB、clean約+3 MiBである。dirty増分の主要項目は以下である。

| Region | Dirty差分 | 解釈 |
|---|---:|---|
| `MALLOC_SMALL` | +3376 KiB | 主に`DefaultMallocZone`。ElwindUI/デモ/AppKitのどれかへの直接帰属は未確定だが、プロセス専有の最優先調査対象。 |
| untagged `VM_ALLOCATE` | +416 KiB | private allocation。allocation tracingなしには所有者未確定。 |
| CoreAnimation | +368 KiB | 詳細mapではSHMが+400 KiB、ALIが-32 KiB。WindowServerと共有されうる小規模な層管理領域。 |
| `__DATA` + `__DATA_DIRTY` | +401 KiB | framework/applicationの書込み可能dataの増分。 |
| stack等 | +80 KiB以下 | 微小。 |
| IOSurface / CoreGraphics | 0 | このFills表示では大きなbacking surface増分は観測されなかった。 |

clean mapped fileの+3 MiBは、主にAppleKeyboardLayouts-L.dat（+1472 KiB）、SFNS.ttf（+400 KiB）、ICU data（+208 KiB）、Helvetica.ttc（+112 KiB）等である。これらはFills UIの文字表示・入力環境に伴うOS file mappingであり、ElwindUIのprivate memory pressure削減の対象ではない。

既存のDケースJSONではimage cache bytesとvector raster cache bytesはともに0である。この採取でもIOSurfaceおよびCoreGraphics差分は0であり、現時点で画像/vector raster cacheや大規模GPU/backing surfaceを主因とする根拠はない。

## 削減候補の優先順位

1. **A — default allocatorの+2.9 MiBを帰属する。** MallocStackLoggingまたはallocation profilerを有効にした別プロセスで、ElwindUIのRenderTree、`Vec`/`HashMap`、graphics-demoのデモデータ、AppKit object allocationを区別する。これはprivate dirtyであり、最大の実測可能な候補である。
2. **B — layer/state数とCoreAnimation +368 KiBの関係を確認する。** 94 live layersに対して増分は小さいため、現状の優先度は低い。layer数を増やすシナリオで比例する場合だけ、PaintIslandやtab detachの効果を測定する。
3. **B — `VM_ALLOCATE` +416 KiBを追跡する。** ownershipが分かるまで削減実装はしない。

mapped file、`__TEXT`、`__LINKEDIT`、dyld shared cache、IOSurface/CGの不変部分をRSSだけを理由に削減対象にしない。今回の結果はRSS 75 MiBを50 MiBにすることではなく、Case Aとの差分で約4–5 MiBのprivate dirtyを下げることが有意義であることを示す。

## MallocStackLoggingによるDefaultMallocZoneの所有者特定

### 方法と解釈上の制約

release buildのCase Aとgraphics-demo（Fills初期タブ）を別プロセスで起動し、`MallocStackLogging=full` と `MallocStackLoggingDirectory` を有効にして、表示後のlive allocationを`malloc_history -allBySize`で集計した。`xctrace`のAllocations templateはこの環境で対象プロセスへのattachに失敗したため、同じ目的に使えるMallocStackLoggingのdisk recorderを用いた。

これは通常起動時の絶対値を再測定するものではない。MallocStackLogging自身が常駐メモリを増やし、両プロセスとも起動後にstack loggingが有効になった旨を`malloc_history`が報告した。そのため、起動直後の一部はstackを持たない。Physical FootprintもCase A 27.8 MiB、graphics-demo 31.9 MiBとなり、通常測定の13.81 MiB / 17.85 MiBより大きい。以下は**同一計測条件内のCase A差分で、live allocationの直接発行者を比較した結果だけ**である。

ここでいうPersistent Bytesは、採取時点まで解放されていないmalloc blockの合計である。allocation stackの最初の非allocator frameで分類した。ElwindUIの呼出し元からAppKitが確保したblockを「ElwindUI直接確保」とは数えず、実際に確保処理を行ったframework/runtime側としている。この区別により、最適化対象を過大に帰属しない。

### Persistent Bytesの分類

| 直接のallocation site分類 | Case A | graphics-demo | graphics-demo - A | 判定 |
|---|---:|---:|---:|---|
| ElwindUI core / AppKit backend | 0 KiB | 12.7 KiB | **+12.7 KiB** | 直接所有はごく小さい |
| graphics-demo | 0 KiB | 4.3 KiB | **+4.3 KiB** | 直接所有はごく小さい |
| AppKit / Foundation / CoreFoundation / CoreAutoLayout等 | 1,940.9 KiB | 2,967.7 KiB | **+1,026.8 KiB** | framework起因 |
| Objective-C / Swift / ICU / libc++ runtime | 2,570.8 KiB | 4,298.5 KiB | **+1,727.7 KiB** | framework起動・UI構築に伴うruntime状態 |
| stack未取得のlive malloc block | 36 KiB | 344 KiB | **+308 KiB** | 起動後にlogging開始したため未帰属 |
| **live malloc block合計** | **4,547.7 KiB** | **7,627.2 KiB** | **+3,079.5 KiB** | DefaultMallocZone増分と整合 |

同じMallocStackLogging条件の`vmmap -summary`では、MALLOC zoneの`ALLOCATED`はCase A 4,801 KiB、graphics-demo 7,925 KiB、差分**+3,124 KiB**だった。stack集計との差約45 KiBは、`malloc_history`の記録開始時点・集計単位・丸めの差であり、約+3 MiBのDefaultMallocZone増分をほぼ説明する。

allocator overheadはPersistent Bytesとは分けて扱う。同じzoneの`FRAG SIZE`はCase A 4,591 KiB、graphics-demo 5,467 KiB、差分**+876 KiB**だった。これはzoneが保持する未割当capacity/fragmentationで、live objectの所有者を示さない。実際のmemory pressureには寄与し得るが、個別のallocation siteへ二重計上せず、A/Bの条件付き削減対象とする。

### 増分の大きい直接allocation site

`graphics-demo - A` のPersistent Bytes差分を、allocatorの直後のframeで正規化して大きい順に示す。これらはdirect siteであり、呼出し経路にはElwindUIのwindow/tree構築が含まれる場合があっても、block自体を確保した実体は右欄のframework/runtimeである。

| allocation site | 増分Persistent Bytes | 分類 |
|---|---:|---|
| `swift::swift_slowAllocTyped` | +452.4 KiB | Swift runtime |
| `_objc_rootAllocWithZone` | +401.4 KiB | Objective-C runtime |
| `swift::MetadataAllocator::Allocate` | +288.0 KiB | Swift runtime |
| `_CFRuntimeCreateInstance` | +171.8 KiB | CoreFoundation |
| `cache_t::insert` | +115.7 KiB | Objective-C runtime |
| `AutoreleasePoolPage::autoreleaseFullPage` | +108.0 KiB | Objective-C runtime |
| `CoreAutoLayout::_table_addStorageBlock` | +104.7 KiB | AppKit/CoreAutoLayout |
| `class_createInstance` | +93.8 KiB | Objective-C runtime |
| `operator_new_impl` | +90.2 KiB | libc++ runtime |
| `AttributeGraph::data::zone::alloc_persistent` | +76.0 KiB | AppKit/SwiftUI support framework |

ElwindUIがcaller chainに現れるlive blockの多くは、`NSApplication`初期化、`InnerWindow::new`、`TreeHostView::set_tree`、`TabView::measure`の過程でframework/runtimeが確保した状態だった。直接siteがElwindUI core/backendである合計は12.7 KiB、graphics-demo固有は4.3 KiBに留まる。したがって、この計測では「約+3 MiBのprivate dirtyをElwindUIのRenderTreeやgraphics-demoデータが直接保持している」と結論付ける根拠はない。

### 所有者の結論と次の判断

- 約+3.0 MiBのlive malloc増分のうち、stackを取得できた約+2.75 MiBはAppKit系frameworkまたはObjective-C/Swift等のsystem runtimeが直接確保したものだった。
- ElwindUI core/backendとgraphics-demoの直接allocation siteは合計約+17 KiBであり、この観測では積極的な削減対象（A）にはならない。
- +308 KiBはstack未取得なので所有者未確定、+876 KiBのallocator fragmentationは個別objectの所有者を持たない。両者は条件付き対象（B）として、将来の実装変更で再計測する。
- AppKit/Foundation/runtime由来のclean/shared mappingではなく、このprivate dirty増分だけを最適化候補の評価対象とする。ただし現時点では所有者特定までとし、最適化実装は行わない。
