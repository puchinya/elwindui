# ElwindUIL Hot Reload設計

本書はsource変更を実行中applicationへ反映するhot reload pipelineと更新粒度を定める。compilerは [`codegen_design.md`](codegen_design.md)、previewとの境界は [`preview_design.md`](preview_design.md)、実装状況は [`../../status/tooling_status.md`](../../status/tooling_status.md)を参照する。

## 1. 更新model

Hot reloadは変更を次のactionへ分類する。

| Action | 条件 | State handling |
|---|---|---|
| Patch | property値、binding式、render内容など既存instanceへ安全に反映できる変更 | compatibleなruntime stateを保持する |
| Remount | construct parameter、tree shape、type、lifecycle boundaryなどinstance identityを再構築する変更 | 対象subtreeを破棄して再生成する |

判定不能な変更をPatchとして扱わない。安全性を証明できない場合はRemountへ寄せる。

## 2. Processing flow

```text
validated source change
        ↓
change classification
        ↓
build reloadable artifact
        ↓
load and compatibility validation
        ↓
Patch or Remount on UI thread
        ↓
retire previous artifact after owners are released
```

compile errorまたはcompatibility errorの場合、実行中の旧artifactを維持する。partial loadをapplication stateへ公開しない。

## 3. Artifact境界

reloadable artifactはstable entrypointとversioned metadataを公開する。hostとartifactはcomponent identity、schema/version、変更分類、construct/patch functionを交換する。Rust ABIへ暗黙に依存せず、reload library integrationが要求する明示的boundaryを用いる。

旧artifactは、そのコードを参照するcallback、subscription、native delegateがreleaseされるまでunloadしない。artifact generationごとのownerをhostが追跡する。

## 4. PatchとRemount

Patchは同一instance identityとcompatible property/event surfaceを前提とする。UI更新は通常のproperty invalidation、subscription、reconciliation経路を通す。

Remountは対象subtreeのsubscription、event handler、native childを通常lifecycleでdetach/dropしてから、新artifactでconstructする。親とのslot identityと配置位置はhostが保持し、置換を一回のUI-thread transactionとして行う。

## 5. Tool連携

Language Serverまたはfile watcherはvalidated changeを通知するが、reload stateを所有しない。previewのlive-application levelは同じhot reload endpointを利用する。build logとreload errorはtool diagnosticとして返し、DSL compile-time diagnosticと区別する。

## 6. Invariants

- buildまたはload失敗時は動作中artifactを保持する。
- unsafeなchangeをPatchへ分類しない。
- UI treeの変更はUI threadで行う。
- callbackやsubscriptionが参照中のartifactをunloadしない。
- public lifecycle semanticsをhot reload専用に変更しない。
- 実装状況はtooling statusに記録する。
