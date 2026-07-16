//! §11(docs/elwindui_spec.md)のFluent(.ftl)ベースi18nの実行時ランタイム。単一ロケール
//! ("en"固定、`strings/en.ftl`)決め打ちの最小実装 — `i18n { default/fallback/available }`の
//! ようなマルチロケール切り替えは対象外。`elwindui-codegen`が生成する`t!(...)`呼び出しは
//! `elwindui::i18n::t`/`elwindui::i18n::FluentValue`を直接参照する(`crates/elwindui/src/lib.rs`の
//! `pub use elwindui_i18n as i18n;`経由)ので、このクレート自体を呼び出し元クレートが
//! `[dependencies]`に持つ必要はない。

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use std::cell::RefCell;

pub use fluent_bundle::FluentValue;

thread_local! {
    static BUNDLE: RefCell<Option<FluentBundle<FluentResource>>> = const { RefCell::new(None) };
}

/// `declare!()`(下記)が展開先で一度だけ呼ぶ。`ftl_source`は呼び出し元クレート自身の`.ftl`
/// ファイルの中身(`include_str!`で埋め込まれたもの)、`lang`はそのロケール識別子("en"等)。
#[doc(hidden)]
pub fn init(ftl_source: &str, lang: &str) {
    let res = FluentResource::try_new(ftl_source.to_string())
        .unwrap_or_else(|(_, errors)| panic!("invalid .ftl file: {errors:?}"));
    let langid: unic_langid::LanguageIdentifier = lang.parse().expect("valid language id");
    let mut bundle = FluentBundle::new(vec![langid]);
    bundle.add_resource(res).expect("adding ftl resource");
    BUNDLE.with(|b| *b.borrow_mut() = Some(bundle));
}

/// 生成コード(`t!("key", arg: value)`)の展開先。`declare!()`が事前に呼ばれている必要がある。
pub fn t(key: &str, args: &[(&str, FluentValue<'_>)]) -> String {
    BUNDLE.with(|bundle| {
        let bundle = bundle.borrow();
        let bundle = bundle.as_ref().unwrap_or_else(|| {
            panic!("elwindui::i18n::t(\"{key}\", ..) called before elwindui::i18n::declare!() ran")
        });
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            fluent_args.set(*name, value.clone());
        }
        let msg = bundle
            .get_message(key)
            .unwrap_or_else(|| panic!("missing fluent message `{key}`"));
        let pattern = msg
            .value()
            .unwrap_or_else(|| panic!("fluent message `{key}` has no value"));
        let mut errors = Vec::new();
        let result = bundle.format_pattern(pattern, Some(&fluent_args), &mut errors);
        result.into_owned()
    })
}

/// 呼び出し元クレート自身の`strings/en.ftl`を読み込んでバンドルを初期化する。`t!(...)`が
/// 評価されるより前に、`main()`冒頭などで一度だけ呼ぶこと。`$crate`によるマクロ衛生のおかげで、
/// 呼び出し元クレートは`elwindui-i18n`自体を`[dependencies]`に持つ必要はない(`elwindui`経由の
/// 間接依存で足りる) — `env!`/`include_str!`はこのマクロが展開された先、つまり呼び出し元クレート
/// のコンパイル時のものとして評価される。
#[macro_export]
macro_rules! declare {
    () => {
        $crate::init(
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/strings/en.ftl")),
            "en",
        );
    };
}
