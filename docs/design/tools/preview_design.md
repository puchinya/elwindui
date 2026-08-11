# ElwindUIL Preview設計

本書はcomponent previewの実行境界と状態modelを定める。compiler連携は [`codegen_design.md`](codegen_design.md)、editor連携は [`languageserver_design.md`](languageserver_design.md)、実装状況は [`../../status/tooling_status.md`](../../status/tooling_status.md)を参照する。

## 1. Preview level

| Level | 目的 | State source |
|---|---|---|
| Static | default値でlayoutとrenderingを確認する | generated default / explicit preview fixture |
| Interactive | bindingとeventをpreview process内で操作する | isolated mock ViewModel |
| Live application | 実際のapplication stateで確認する | running application and hot reload boundary |

最初の2 levelはpreview subsystemが所有し、live applicationへの反映は [`hotreload_design.md`](hotreload_design.md)へ委譲する。

## 2. Processing flow

```text
validated component + preview fixture
                  ↓
isolated preview process
                  ↓
component construction and layout
                  ↓
offscreen render / interaction channel
                  ↓
editor preview panel
```

previewはmain application processと分離し、invalid input、panic、native backend failureがeditor本体を終了させない。requestにはdocument versionとpreview instance IDを付け、古いframeやeventを破棄する。

## 3. Fixtureとmock state

`#[param]` などconstruct時に必要な値はexplicit preview fixtureを優先し、値がない場合だけ型のdefault policyを利用する。meaningfulな値をtoolが推測してcontract化しない。

Interactive previewのmock ViewModelはproperty、subscription、event surfaceを保つ。production data source、network、filesystemなどの外部副作用は自動実行せず、明示されたpreview adapterへ置き換える。

## 4. Rendering boundary

preview processは通常のruntime tree、layout、rendering pipelineを使用する。preview専用のlayout semanticsやcontrol behaviorを作らない。backend固有native controlがoffscreen表示を必要とする場合も、通常backend adapterを介して構築する。

frameはimageまたはplatform-neutral drawing resultとしてeditorへ渡す。hit testing用geometryとaccessibility metadataはinteractive event routingに必要な範囲で同じframe versionへ関連付ける。

## 5. Invariants

- preview behaviorでpublic contractを上書きしない。
- editor processとpreview executionを分離する。
- production external side effectを暗黙に実行しない。
- document version、instance ID、frame versionを混同しない。
- 実装状況はtooling statusに記録する。
