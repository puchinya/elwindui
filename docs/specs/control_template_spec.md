# ControlTemplate specification

Tracking: [#83](https://github.com/puchinya/elwindui/issues/83)

## 1. Scope

`ControlTemplate<C>`は、ElwindUIがVisual subtreeを所有する`Control`派生型の外観を、
mount時に確定した`EnvironmentContext`から選択するtyped template valueである。

`NativeControl`はplatform backendがVisual構造を所有するため対象外である。
初期版はmount lifetime中のtemplate差し替えを行わない。

## 2. Core API

```rust
#[derive(Clone)]
pub struct ControlTemplate<C: ControlExt + 'static> {
    // private typed factory
}

pub struct ControlTemplateContext<C: ControlExt + 'static> {
    pub control: Rc<C>,
    pub environment: EnvironmentContext,
}

impl<C: ControlExt + 'static> ControlTemplate<C> {
    pub fn new(
        factory: impl Fn(ControlTemplateContext<C>) -> Rc<dyn UIElementExt> + 'static,
    ) -> Self;
}
```

factoryはcapturing closureを保持できる。factoryへ渡す`Rc<C>`はbuild call中のtyped targetであり、
template implementationはtargetとtemplate rootのstrong ownership cycleを作ってはならない。

## 3. Template-enabled component

replaceable templateを持つComponentは、使用するtyped Environment Keyを明示する。

```rust
#[elwindui::environment_key(
    name = rounded_panel_template,
    value = Option<ControlTemplate<RoundedPanel>>,
    default = None
)]
pub struct RoundedPanelTemplateEnvironment;

#[elwindui::component(
    inherits Control,
    template = rounded_panel_template
)]
struct RoundedPanel {
    body: view! { /* default template */ },
}
```

KeyのValue型は`Option<ControlTemplate<Component>>`でなければならない。
`None`はComponentの`body: view!`をdefault templateとして使うことを意味する。

## 4. Selection and lifecycle

template selectionはlogical constructionでは行わず、次の順序で一度だけ行う。

```text
logical construction
  -> mount(effective Environment)
  -> own Environment fields resolve
  -> template Key resolve
  -> selected template subtree build and mount
  -> template wiring
  -> target Control wiring
  -> target Control on_mount
```

初期版の優先順位は次の通りである。

1. Environment Keyの`Some(ControlTemplate<Component>)`
2. Componentの`body: view!` default template

custom templateを選択した場合、default bodyのVisual node、binding、subscriptionを構築してはならない。
template subtreeはtargetと同じeffective `EnvironmentContext`を継承する。
Environment Keyの変更はruntime re-templateを発生させない。

## 5. Authoring and templated_parent

```rust
#[elwindui::control_template(target = RoundedPanel)]
struct CompactRoundedPanelTemplate {
    body: view! {
        Border {
            TextBlock { text: templated_parent.label }
        }
    },
}
```

authoring typeは`CompactRoundedPanelTemplate::template() -> ControlTemplate<RoundedPanel>`を提供する。
`templated_parent`はtarget型へのtyped ownerであり、通常のgetterと既存PropertyChanged wiringを使う。
property変更は参照しているtemplate要素だけをresyncし、template factory自体は再実行しない。

replaceable template内の`#[id]`は初期版では禁止する。custom templateに同じpartが存在する保証を
提供する`TemplatePart` contractは本仕様の対象外である。

## 6. ContentControl and ContentPresenter

template-enabled `ContentControl`派生型では、logical contentとtemplate rootを別のedgeとして扱う。

```text
ContentControl
  |- logical content
  `- visual template root
       `- ContentPresenter
            `- visual reference to logical content
```

`ContentPresenter`はtemplated parentのlogical contentをtemplate内の位置へVisual表示するだけであり、
contentのlogical parentを変更しない。`set_content`による置換はactive presenterへ反映され、old contentは
logical/Visual両edgeから外れ、new contentは一つのVisual parentだけを持つ。

templateは静的な`ContentPresenter`を0個または1個持てる。0個の場合contentはlogical contentとして
保持されるが表示されない。複数配置またはdynamic region内の配置は初期版ではcompile-time errorである。

replaceable templateを使用しないraw `ContentControl`のdirect presentationは互換性のため維持する。

## 7. Validation

可能な限りuser codeを指すcompile-time diagnosticを出す。

- template-enabled componentとauthoring targetは`ControlExt`を実装しなければならない。
- `NativeControl`および非`Control` targetはrejectする。
- template Keyが未登録、またはValue型が一致しない場合はrejectする。
- bodyの欠落・重複、不正なtarget、replaceable body内の`#[id]`をrejectする。
- `ContentPresenter`の複数配置とdynamic配置をrejectする。

cross-crate propertyとtarget型の最終検証は生成Rustのtrait bound/getter解決をrustcへ委ねる。
public `Any` erasure、unchecked downcast、runtime string target lookupは使用しない。

## 8. Non-goals

- per-instance `template:` property
- runtime re-templateとtransition
- NativeControl template、native Style
- TemplatePart、VisualStateManager、Trigger
- ResourceDictionary、runtime Binding API、reflection、string property path
- full Virtual DOMまたはtemplate body diffing
