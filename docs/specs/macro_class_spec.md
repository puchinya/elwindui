# `#[elwindui_macros::class]` Specification

## 1. Scope

本仕様は `#[class]` を利用する側から観測できるclass形態、継承、生成API、override、construction、compile-time diagnosticsを定義する。macro expansionの内部方式は [`../design/tools/class_macro_design.md`](../design/tools/class_macro_design.md) を参照する。

## 2. Class forms

`#[class]` は次の形態を公開する。

| Form | Declaration | Meaning |
|---|---|---|
| ordinary class | paired `struct` and inherent `impl` | storageと公開extension surfaceを持つclass |
| root class | ordinary class without an ancestor | hierarchyのroot |
| `trait_only` | trait-like interface declaration | storageを生成せずinterfaceだけを定義 |
| `struct_only` | concrete storage implementing an existing class interface | 新しいper-class extension surfaceを追加しない具象実装 |
| abstract class | non-instantiable class declaration | descendantの共通contractを定義 |

同じclass名に必要な宣言が複数ある形態では、macroがそれらを一つのclass contractとして検証する。

## 3. Class arguments

- `inherits = Path` は一つの直接ancestorを指定する。
- `implements(...)` は追加interface contractを指定する。
- `supertrait(...)` は生成されるextension traitの公開supertrait制約を指定する。
- `trait_only`、`struct_only`、root/abstract/sealedに関する指定は互いに矛盾しない組み合わせでなければならない。
- class、ancestor、interfaceはmacro expansion地点から解決可能なpathで指定する。

不明なargument、重複した単一値argument、矛盾するclass formはcompile-time errorとなる。

## 4. Generated public API

ordinary/root classは、classの公開操作をdyn dispatch可能にするextension traitと、そのclass storageへのaccess pathを生成する。ancestorの公開操作はdescendant handleから利用できる。

`trait_only` はinterfaceを公開するがstorage/constructorを生成しない。`struct_only` は指定された既存interfaceを実装するが、ordinary classと同じ独自override accessorを生成するとは限らない。ただし `struct_only` は、実装するinterfaceのoverride契約(§5)に対して透過的である — `struct_only` implementorを経由した descendant は、そのinterfaceの `#[overridable]` slotを通常の `#[overrides]` でoverrideできる(§5参照)。

生成名はmacroのpublic contractの一部であり、利用側crateやmoduleを跨いでも同じclass chainとして解決されなければならない。

## 5. Methods and override semantics

- override可能な基底methodは `#[overridable]` として宣言する。
- descendant実装は `#[overrides]` を付け、同じ公開signatureでancestor slotを置き換える。
- sealed method/classはその地点より下でoverrideできない。
- override dispatchは最も具体的な実装を選び、明示的なancestor forwardingは次の実装へ委譲する。
- ordinary inherent methodでoverride chainへ参加しないものは通常のRust methodとして扱う。

`#[overridable]` / `#[overrides]` methodは `&self` receiverとmacroが対応するplain argumentを使う。generic、`where`、`async`、`unsafe`、trait `impl`等の非対応形はcompile-time errorとなる。

`struct_only` は新しいper-class traitを持たないため、そのclassだけの `#[overridable]` slotを追加できない。overrideが必要な操作は既存interface側で定義する。

`struct_only` implementorはこのoverride契約に対して**透過(transparent)**である: あるinterfaceが `#[overridable]` として宣言したmethodは、そのinterfaceを実装する `struct_only` classを経由したordinary descendant chainのどの深さからでも、通常の `#[overrides]` で置き換えられる — chainの途中に `struct_only` implementorが挟まっていることをdescendant側が意識する必要はない。この透過性は:

- 任意のordinary descendant深さで成立する(2ホップに限定されない);
- `struct_only` implementorの宣言crateとinterfaceの宣言crateが異なっていても成立する;
- `struct_only` implementorの具象型名(bare name)は、実装するinterfaceの名前と一致している必要はない — `Window`/`NativeControl`のように名前が一致する例が多いのは既存の命名慣習であって言語上の要件ではなく、名前が異なる場合でも手書きの互換aliasは一切不要である;
- override dispatchの「最も具体的な実装が勝つ」規則、および明示的なancestor forwardingの意味論を変更しない(§5冒頭の規則がそのまま適用される)。

`no_ancestor_forward` を指定した `struct_only`(手書きの既存traitを対象とする、`__dyn_x` 規約に従わない特殊形)は、このoverride透過性の対象外のままである — 挙動は変更されない。

root class(`inherits`を持たない、`UIElement`など)もこの透過性の生成対象ではある(自身のclass-interface bridge macroを生成する)が、root classの `as_ui_element` はdeclaring struct自身の具象型に固定されたreturn typeを持つ必須trait methodであるため、root interfaceに対する実際にruntimeで動作する `struct_only` implementorは原理的に成立しない。これはbridge機構自体の制約ではなく、root modeの既存設計そのものの性質である。

## 6. Construction

- instantiable ordinary/root classは `construct` をconstructor inputの正本として宣言する。
- macroが公開 `new` を生成するため、同じclassでhand-written `new`を競合させてはならない。
- object identityとweak self handleは、`on_constructed`が呼ばれる時点で利用可能でなければならない。
- abstract classと `trait_only` は直接構築できない。

required `construct`の欠落、競合する `new`、生成constructorと一致しない宣言はcompile-time errorとなる。

## 7. Inheritance and conversion

descendantは直接ancestorだけでなく完全なancestor chainの公開class contractを満たす。ancestor field/accessorの生成形は内部実装だが、次は公開上保証される。

- ancestor method/propertyがdescendant handleから利用できること;
- override dispatchがcrate/module境界で変化しないこと;
- sealed restrictionが境界を跨いでも維持されること;
- UIElement ancestorを持つclassだけがgeneric UIElement childとして変換可能であること。

## 8. Path and module requirements

macroが生成する公開itemは、宣言moduleの子moduleおよび別crateから通常のvisibility規則に従って利用できる。利用側は内部helper macroやregistryの存在を前提にしてはならない。

ancestor/interface pathが解決不能、必要なgenerated public itemがvisibility上利用不能、または異なるclass chainとして解釈される宣言はcompile-time errorとなる。

## 9. Diagnostics

macroは少なくとも次を宣言地点へ関連付けて診断する。

- unsupported target item or class form;
- missing/duplicate paired declaration;
- invalid or conflicting class arguments;
- unsupported method tag/signature;
- `#[overrides]` without a matching overridable ancestor;
- override of a sealed member;
- missing construction contract or conflicting constructor;
- unresolved/inaccessible ancestor or interface path.

rust-analyzer用shadow、token rewrite、registry、span propagationの実装は公開contractではない。
