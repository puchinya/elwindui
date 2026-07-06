# ElwindUIL ビルトイン部品仕様書

`docs/elwindui_spec.md`(言語コア仕様)から、`builtin::`名前空間のUI要素・`platform::`名前空間のOS機能アクセスに関する付録を分離したもの。
元の付録番号(付録F・G・L・M・N・Q・T)はそのまま維持しており、`elwindui_spec.md`側に残る「付録G参照」等の記述は本ファイル内の該当節を指す。
付録X・Yはこのファイルで新規に追加したビルトイン(MenuBar・TabView)。

言語コア(`component`/`view`/`param`/`prop`/`Element`トレイト・14章の静的検証ルール等)は引き続き`docs/elwindui_spec.md`が正とする。

---

# 付録F. 標準ビルトイン部品のリファレンス実装

`Window`, `VerticalLayout`/`HorizontalLayout`, `Text`, `TextArea`, `Dropdown`/`Option` など、これまで暗黙に使ってきたビルトインプリミティブは、実際には `builtin` 名前空間(付録E参照)に属し、コード生成器が標準で提供する。ネイティブな葉ウィジェット(`Window`/`Button`/`TextArea`/`Text`/`MenuBar`/`TabView`等)は他のコンポーネントと同じ`component`/`view`構文で表現でき、`match target::backend()`による網羅性検査(付録D)や`native!`エスケープハッチ(付録A・C)がそのまま適用される。一方`VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`のような仮想ビルトインは専用のネイティブ実体を持たず、`elwindui_core::tree::Node::Virtual`として`elwindui-codegen`が直接組み立てる(F.2参照)。

## F.1 `builtin::Window`

```rust
enum Direction { Ltr, Rtl }

component Window {
    title: String,
    #[param]
    width: number = 800,
    #[param]
    height: number = 600,
    #[param]
    direction: Direction = env::direction(),
    children: Vec<Element>,
}

view Window {
    match target::backend() {
        Backend::Winui3 => native! {
            let win = Microsoft::UI::Xaml::Window::new()?;
            win.SetTitle(&title)?;
            win.SetFlowDirection(direction)?;
            win.SetContent(&build_children(children))?;
            win
        }
        Backend::Appkit => native! {
            let win = NSWindow::init_with_size(width, height);
            win.setTitle(&text::from(title));
            win.setContentView(&build_children(children));
            win
        }
        Backend::Gtk4 => native! {
            let win = gtk::ApplicationWindow::new(&app);
            win.set_title(Some(&title));
            win.set_default_size(width as i32, height as i32);
            win.set_child(Some(&build_children(children)));
            win
        }
        Backend::Egui | Backend::Iced => native! {
            egui::Window::new(&title)
                .default_size([width, height])
                .show(ctx, |ui| { render_children(ui, &children); })
        }
    }
}
```

## F.2 `builtin::VerticalLayout` / `builtin::HorizontalLayout`

`VerticalLayout`/`HorizontalLayout`(かつての`Column`/`Row`は機能が完全に重複するため廃止された)は、
**専用のネイティブ実体を一切持たない**。バックエンドごとの`component`+`view`ペアや`native!`分岐は
存在せず、`.elwind`側は以下のシェイプ宣言のみで完結する:

```
component VerticalLayout {
    #[param]
    children: Vec<AnyView>,
    #[param]
    spacing: Option<f32>,
    #[param]
    cross_align: Option<CrossAlign>,
}
```

(`HorizontalLayout`も同じ形。実ファイルは`elwindui-builtins/src/shapes/{vertical_layout,horizontal_layout}.elwind`)

`elwindui-codegen`(`is_virtual_builtin`/`emit_virtual_construction`)が、使用箇所ごとに直接
以下のような値を組み立てる——`Type::new(..)`という関数呼び出しは一切発生しない:

```rust
elwindui_core::tree::Node::Virtual {
    content: Box::new(elwindui_core::tree::Stack {
        orientation: elwindui_core::layout::Orientation::Vertical, // または Horizontal
        spacing: /* spacing属性、省略時は0.0 */,
        cross_align: /* cross_align属性、省略時はStretch */,
    }),
    children: /* 子要素を同じ規則で再帰的にNode<AnyView>化したもの */,
}
```

実際にこの値をネイティブsubviewとして配置するのは、祖先のネイティブコンテナ(`Window`や`TabView`)
が持つ、任意の`Node<AnyView>`を受け付ける単一の再利用可能なホスト(AppKitの`TreeHostView`、
WinUI3の`TreeHostPanel`)であり、`VerticalLayout`/`HorizontalLayout`自体はバックエンドコードを
一切持たない。新しいレイアウト種別(将来の`Grid`等)を追加する際も、
`elwindui_core::tree::VirtualNode`トレイトの実装を1つ足すだけでよく、バックエンドごとの
`native!`分岐を増やす必要はない(詳細は`elwindui-core/src/tree.rs`のモジュールコメントを参照)。

## F.3 `builtin::Text`

```rust
component Text {
    text: String,
    #[param]
    color: ColorHex? = None,
}

view Text {
    match target::backend() {
        Backend::Winui3 => native! {
            let tb = microsoft::ui::xaml::controls::TextBlock::new()?;
            tb.SetText(&text)?;
            if let Some(c) = color { tb.SetForeground(&brush_from(c))?; }
            tb
        }
        Backend::Appkit => native! {
            let label = NSTextField::labelWithString(&text);
            if let Some(c) = color { label.setTextColor(&nscolor_from(c)); }
            label
        }
        Backend::Gtk4 => native! {
            let lbl = gtk::Label::new(Some(&text));
            if let Some(c) = color { apply_css_color(&lbl, c); }
            lbl
        }
        Backend::Egui | Backend::Iced => native! {
            match color {
                Some(c) => ui.colored_label(egui_color(c), &text),
                None    => ui.label(&text),
            }
        }
    }
}
```

## F.4 `builtin::TextArea`

```rust
component TextArea {
    text: String = bind!(self.text, TwoWay),
    #[param]
    padding_start: number = 0,
    #[param]
    padding_end: number = 0,
}

view TextArea {
    match target::backend() {
        Backend::Winui3 => native! {
            let box_ = microsoft::ui::xaml::controls::TextBox::new()?;
            box_.SetAcceptsReturn(true)?;
            box_.SetText(&text)?;
            box_.TextChanged(&TextChangedHandler::new(move |new_text| { text = new_text; }))?;
            box_
        }
        Backend::Appkit => native! {
            let view = NSTextView::new();
            view.setString(&text);
            view.set_delegate_on_change(move |new_text| { text = new_text; });
            view
        }
        Backend::Gtk4 => native! {
            let buf = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
            buf.set_text(&text);
            let tv = gtk::TextView::with_buffer(&buf);
            buf.connect_changed(move |b| { text = b.text(&b.start_iter(), &b.end_iter(), false).to_string(); });
            tv
        }
        Backend::Egui | Backend::Iced => native! {
            ui.add(egui::TextEdit::multiline(&mut text))
        }
    }
}
```

## F.5 `builtin::Dropdown` / `builtin::Option`

```rust
component Option {
    text: String,
    #[param]
    selected: bool = false,
}

component Dropdown {
    options: Vec<Option>,
}

view Dropdown {
    match target::backend() {
        Backend::Winui3 => native! {
            let combo = microsoft::ui::xaml::controls::ComboBox::new()?;
            for opt in &options { combo.Items().Append(&opt.text)?; }
            combo.SetSelectedIndex(find_selected_index(&options))?;
            combo
        }
        Backend::Appkit => native! {
            let popup = NSPopUpButton::new();
            for opt in &options { popup.addItemWithTitle(&opt.text); }
            popup.selectItemAtIndex(find_selected_index(&options));
            popup
        }
        Backend::Gtk4 => native! {
            let model = gtk::StringList::new(&options.iter().map(|o| o.text.as_str()).collect::<Vec<_>>());
            let dd = gtk::DropDown::new(Some(model), gtk::Expression::NONE);
            dd.set_selected(find_selected_index(&options) as u32);
            dd
        }
        Backend::Egui | Backend::Iced => native! {
            egui::ComboBox::from_id_source("dropdown")
                .selected_text(current_selected_text(&options))
                .show_ui(ui, |ui| {
                    for opt in &options { ui.selectable_label(opt.selected, &opt.text); }
                })
        }
    }
}
```

## F.6 図形プリミティブ(`builtin::Rectangle` / `builtin::Ellipse`)について

かつてここには`builtin::Rect`(egui/iced backend向けにButtonが代替表現として使っていたクリック可能な
最小コンテナ)という節があったが、egui/icedバックエンド自体が既に削除されており、この概念に対応する
実装はコードベースのどこにも存在しない(仕様書にのみ残っていた設計と思われる)ため削除した。

現在の図形プリミティブは`Rectangle`/`Ellipse`であり、F.2の`VerticalLayout`/`HorizontalLayout`と
全く同じ仕組み(専用のネイティブ実体を持たず、`elwindui-codegen`が`elwindui_core::tree::Node::Virtual{content: Box::new(elwindui_core::tree::Shape{..}), ..}`を直接組み立てる)で実装されている。
詳細はG章・N章(Canvas/Painterによるカスタム描画)を参照。
```

## F.7 部品の全体依存関係(メモ帳の例)

```
NotepadWindow
 ├─ Window
 │   └─ VerticalLayout
 │       ├─ HorizontalLayout
 │       │   ├─ ToolbarButton → Button(#[overrides])
 │       │   └─ Dropdown → Option
 │       ├─ TextArea
 │       └─ StatusBar
 │           └─ HorizontalLayout → Text
```

## F.8 各部品で使われている仕様の対応

| 部品 | 使用している仕様 |
|---|---|
| `Window` | `#[param] direction = env::direction()`、`match target::backend()`の網羅性検査 |
| `VerticalLayout`/`HorizontalLayout` | 専用のネイティブ実体を持たない仮想ツリー(`elwindui_core::tree::Node::Virtual` + `VirtualNode`実装の`Stack`) |
| `Text` | `ColorHex?`(nullable制約)、backendごとのカラー変換 |
| `TextArea` | `bind!(self.text, TwoWay)`による双方向バインディング |
| `Dropdown` / `Option` | `Vec<Option>`という複合型プロパティ、backendごとの選択状態同期 |

これらの標準ビルトイン実装は、通常はコード生成器(`elwindui-codegen`)が内部に持ち利用者が直接編集する必要はないが、`#[overrides(builtin::X)]`(付録E)を使うことで、プロジェクト固有の要件に応じて安全に差し替えられる。

---

# 付録G. 独自描画部品(Canvas / Painter)

グラフ・ゲージ・独自アニメーションのような「ピクセル単位で自分で描く」部品は、既存部品の組み合わせでは表現できない。これは`view`の宣言的な要素ツリー構文の対象外とし、**`Canvas`という専用ビルトインとRustの命令的な描画コードの組み合わせ**として扱う。

## G.1 基本方針

- レイアウト(どこに何を置くか)は引き続き宣言的な`.elwind`で書く
- 描画内容(何をどう塗るか)は宣言的に書かず、`Painter`という抽象描画APIを受け取るRust関数として書く
- `Painter`はバックエンドごとの実描画API(Direct2D/Win2D, Core Graphics, Cairo, egui::Painter等)を裏で呼ぶ薄い抽象化層であり、`elwindui-core`(付録H参照)に属する

```rust
use painters::volume_meter::draw_meter;

component VolumeMeter {
    #[range(0..=1)]
    level: f64,
}

view VolumeMeter {
    Canvas {
        width: 200
        height: 40
        on_paint: draw_meter(painter, level)
    }
}
```

## G.2 `Painter` 抽象APIとバックエンド対応

```rust
trait Painter {
    fn fill_rect(&mut self, rect: Rect, color: Color);
    fn stroke_rect(&mut self, rect: Rect, color: Color, width: f32);
    fn stroke_circle(&mut self, center: Point, radius: f32, color: Color, width: f32);
    fn draw_line(&mut self, from: Point, to: Point, color: Color, width: f32);
    fn draw_path(&mut self, path: &Path, style: PaintStyle);
    fn draw_text(&mut self, text: &str, pos: Point, font: Font, color: Color);
    fn draw_image(&mut self, image: &Image, rect: Rect);
}
```

上記は基本図形のみの最小セットである。グラデーション・シャドウ・変形・アニメーション等、WinUI3の`Composition`/`Win2D`相当の機能一式は付録Nで`Painter`を拡張する形で定義する。

| Painterメソッド | WinUI 3 | AppKit | GTK4 | egui |
|---|---|---|---|---|
| `fill_rect` | Win2D `CanvasDrawingSession::FillRectangle` | Core Graphics `CGContextFillRect` | Cairo `cairo_rectangle`+`fill` | `egui::Painter::rect_filled` |
| `draw_line` | Win2D `DrawLine` | `CGContextStrokeLineSegments` | `cairo_move_to`/`line_to` | `Painter::line_segment` |
| `draw_text` | `CanvasTextLayout` | `NSAttributedString::draw` | Pango経由 | `Painter::text` |

`builtin::Canvas`自身は付録Fの他部品と同様、`match target::backend()`で各バックエンドのネイティブ描画コンテキストを`Painter`実装でラップして`on_paint`に渡す(バックエンド分岐が許されるのは`builtin`定義のみという原則はここでも維持される)。

```rust
component Canvas {
    #[param]
    width: number,
    #[param]
    height: number,
    on_paint: fn(&mut Painter),
}

view Canvas {
    match target::backend() {
        Backend::Winui3 => native! {
            let ctrl = microsoft::ui::xaml::controls::CanvasControl::new()?;
            ctrl.Draw(&DrawHandler::new(move |session| {
                let mut p = Win2DPainter::wrap(session);
                on_paint(&mut p);
            }))?;
            ctrl
        }
        Backend::Egui => native! {
            let (response, mut painter) = ui.allocate_painter(egui::vec2(width, height), egui::Sense::hover());
            let mut p = EguiPainter::wrap(&mut painter);
            on_paint(&mut p);
        }
        _ => native! { /* Appkit / Gtk4 も同様にラップ */ }
    }
}
```

## G.3 独自部品はバックエンド共通実装に限定する(重要ルール)

**バックエンド分岐(`native!`/`match target::backend()`)を書けるのは`builtin`定義と`#[overrides(builtin::X)]`が付いたコンポーネントだけ**とする(14章ルール9)。通常の独自部品は常にビルトイン要素の組み合わせ、または`Canvas`+`Painter`のみで実装する。

| コンポーネント種別 | バックエンド分岐の可否 |
|---|---|
| `builtin::*`(付録F) | 可(各OSネイティブAPIを直接呼ぶ) |
| `#[overrides(builtin::X)]`(付録E) | 可(ビルトインの置き換えという性質上必要) |
| 通常の独自部品 | 不可。常にバックエンド共通実装のみ |

**どうしてもネイティブAPIが必要だと感じた場合の判断フロー:**

```
独自部品を書いていて native! が必要だと感じたら:

  Q. これは既存ビルトインの代替実装か?
     YES → #[overrides(builtin::X)] として定義し直す(付録E)
     NO  → Canvas + Painter で表現できないか再検討する
           それでも無理な場合のみ、新しいビルトインをフレームワーク側に追加提案する
```

これにより、バックエンド分岐が必要な箇所は`builtin`一箇所に集約され、ユーザーが書く独自部品のコードベースにはバックエンド分岐が一切現れない状態を維持できる。

## G.4 描画コードのRustファイル分離

`on_paint`のようなコールバックは`on_click`と同じく、`.elwind`側は関数参照のみを持ち、実装は通常のRustファイルに分離する。

```rust
// src/painters/volume_meter.rs (通常のRustファイル、.elwindの外)
use elwindui::painter::{Painter, Rect, Color};

pub fn draw_meter(p: &mut Painter, level: f64) {
    p.fill_rect(Rect::new(0.0, 0.0, 200.0, 40.0), Color::hex("#eeeeee"));
    p.fill_rect(Rect::new(0.0, 0.0, 200.0 * level, 40.0), Color::hex("#2ecc71"));
    p.stroke_rect(Rect::new(0.0, 0.0, 200.0, 40.0), Color::hex("#999999"), 1.0);
}
```

**推奨ディレクトリ構成:**

```
src/
├── ui/                       # .elwind本体(レイアウト定義)
│   ├── notepad_window.elwind
│   └── volume_meter.elwind
├── painters/                 # 描画ロジック(通常のRust、バックエンド共通実装)
│   ├── volume_meter.rs
│   ├── knob.rs
│   └── mod.rs
└── logic/                    # on_click等の業務ロジック
    └── document.rs
```

`Painter`が既にバックエンド差異を吸収しているため、`painters/*.rs`は原則1ファイル1実装で全バックエンドに対応できる。`use painters::volume_meter::draw_meter;` は12章の`use`構文をそのまま使い、参照先が`.elwind`か`.rs`かはパスからコンパイラが自動判別する。

`Painter`で表現しきれないネイティブ専用描画がどうしても必要な場合のみ、`painters/<name>/`をディレクトリ化しRust標準の`#[cfg(feature = "backend-...")]`で分岐する。これは`.elwind`の文法ではなく通常のRustコード側の関心事であるため、`target::backend()`ではなくRust標準のcfg機構を使う。

## G.5 再描画のトリガーとアニメーション

`Canvas`の`prop`(例:`level`)が変わったら、通常の`prop`更新の仕組み(4章)と同じルールで再描画がトリガーされる。

毎フレーム再描画したいアニメーションの場合は`#[animated]`を付け、propの変化を待たず常に再描画対象にすることを明示する。

```rust
Canvas {
    #[animated]
    on_paint: draw_spinner(painter, elapsed_time())
}
```

`#[animated]`が付いた`on_paint`内でのみ、`elapsed_time()`のような非純粋関数の呼び出しが許可される(14章ルール2の例外)。

## G.6 インタラクション(クリック・ドラッグ)

```rust
component DraggableKnob {
    #[range(0..=1)]
    value: f64,
}

view DraggableKnob {
    Canvas {
        width: 60
        height: 60
        on_paint: draw_knob(painter, value)
        on_pointer_down: |pos| start_drag(pos)
        on_pointer_move: |pos| value = knob_value_from_pos(pos)
    }
}
```

座標系はDPIスケーリングを吸収した論理ピクセル座標に統一し、`Painter`実装側がバックエンドごとの実ピクセル変換を担う。

## G.7 混載部品(既存部品 + Canvas)

`Row`/`Column`/`Text`/`Button`のようなビルトイン部品と`Canvas`は、同じ`view`ブロック内で自由に混在できる。G.3のルールがそのまま効くため、混載していても全体がバックエンド共通のまま保たれる。

```rust
use painters::knob::draw_knob;

component VolumeSlider {
    #[range(0..=1)]
    value: f64,

    #[computed]
    percent_label: String = format!("{}%", (value * 100.0) as i32),
}

view VolumeSlider {
    Row {
        spacing: 12

        Text { text: t!("volume-label") }

        Canvas {
            width: 60
            height: 60
            on_paint: draw_knob(painter, value)
            on_pointer_move: |pos| value = knob_value_from_pos(pos)
            #[accessible(role: Slider, label: t!("a11y-volume"), value: percent_label)]
        }

        Text { text: percent_label }

        Button {
            text: t!("volume-mute")
            on_click: value = 0.0
        }
    }
}
```

混載が問題なく成立する理由:

| 仕組み | 混載を支えている理由 |
|---|---|
| `Element`トレイト(13章) | `Canvas`も`Row`も同じ`Element`として扱われ、ツリー上の位置づけに差がない |
| `LayoutNode`(付録H) | `Canvas`は「指定サイズを占有するノード」として他の部品と同じレイアウト計算に参加する |
| `Painter`抽象(本付録) | `Canvas`内部の描画がバックエンド非依存なので、混載してもバックエンド分岐が漏れ出さない |
| G.3のバックエンド分岐禁止ルール | 混載した`view`全体を見てもnative!が現れないため、静的検証にそのまま合格する |
| `#[accessible(...)]`推奨(付録H) | `Canvas`部分だけ明示的なアクセシビリティ情報が必要という区別が保たれ、混載時も漏れなく検証できる |

## G.8 まとめ

| 要件 | 対応 |
|---|---|
| グラフ・ゲージ等の独自描画 | `Canvas` + `on_paint: fn(&mut Painter)` |
| バックエンド間の描画API差異の吸収 | `Painter`トレイトと各backendのラッパー実装(`builtin::Canvas`内部のみ) |
| 独自部品はバックエンド共通実装に限定 | `native!`/`target::backend()`の使用を通常のcomponentでは静的エラーとする(14章ルール9) |
| propに連動した再描画 | 既存の`prop`更新ルール(4章)をそのまま流用 |
| 常時アニメーションさせたい | `#[animated]`で毎フレーム再描画対象と明示、非純粋関数呼び出しを許可 |
| クリック・ドラッグ等の入力 | `on_pointer_down`/`on_pointer_move`等のコールバックをCanvasに追加 |
| 描画コードの分離 | `painters/*.rs`にバックエンド共通実装を配置、`use`で参照 |
| 既存部品との混載 | 同じ`Element`ツリー・`LayoutNode`として自然に共存可能 |


---

# 付録L. 画面遷移(ナビゲーション)

複数画面を持つアプリのための、ルート(画面種別)ベースのナビゲーション機構。`NavigationHost`はビルトインとして提供される。

## L.1 ルート定義

```rust
enum Route {
    Main,
    Settings,
    Search,
}
```

## L.2 `NavigationHost`

```rust
component App {
    #[param]
    current_route: Route = Route::Main,
}

view App {
    NavigationHost {
        route: current_route

        match current_route {
            Route::Main     => NotepadWindow { }
            Route::Settings => SettingsWindow { }
            Route::Search   => SearchDialog { }
        }
    }
}
```

- `match current_route { ... }` は`Route` enumの全メンバーを網羅していなければ静的エラーになる(8章の網羅性検査、14章ルール14)
- `NavigationHost`はビルトインのため、内部で`match target::backend()`によるバックエンド別実装を持つ(付録G.3の原則通り、通常のcomponentではこの分岐は書けない)

| バックエンド | 実装 |
|---|---|
| WinUI3 | `Microsoft::UI::Xaml::Controls::Frame`によるページ遷移 |
| AppKit | `NSWindow`の`contentViewController`差し替え、またはシート/ウィンドウ切り替え |
| GTK4 | `gtk::Stack` + `gtk::StackTransitionType` |
| egui / iced | 内部状態による表示切り替え(単一ウィンドウ内でツリーの入れ替え) |

## L.3 遷移操作

```rust
fn open_settings() {
    navigate!(Route::Settings);
}

fn go_back() {
    navigate_back!();
}
```

- `navigate!(route)` — 指定ルートへ遷移し、遷移履歴に積む
- `navigate_back!()` — 履歴を1つ戻す(履歴が空の場合は何もしない)
- これらはマクロ呼び出し形式(10章の`bind!`と同じ慣習)であり、`NavigationHost`の内部履歴スタックを操作する

## L.4 まとめ

| 要件 | 対応 |
|---|---|
| 複数画面の定義 | `enum Route { ... }` |
| ルートに応じた画面切り替え | `NavigationHost { route, match route { ... } }` |
| 遷移漏れの検出 | `match`の網羅性検査(14章ルール14) |
| 遷移操作 | `navigate!(route)` / `navigate_back!()` |
| バックエンドごとの実装差 | `NavigationHost`内部にのみバックエンド分岐を許可(付録G.3の原則を維持) |

---

# 付録M. ダイアログ・ポップアップ・メニュー

メインウィンドウの外に浮く一時的なUI(モーダルダイアログ、コンテキストメニュー、ツールチップ)のためのビルトイン部品。

## M.1 `Dialog`(モーダル)

```rust
component App {
    #[param]
    show_settings: bool = false,
}

view App {
    NotepadWindow { }

    if show_settings {
        Dialog {
            title: t!("settings-title")
            on_dismiss: show_settings = false

            SettingsPanel { }
        }
    }
}
```

- `Dialog`はビルトインで、`#[focus(trap: true)]`(付録H.3)が自動的に適用される。ダイアログ表示中はTabキーによるフォーカス移動がダイアログ内に閉じ込められる
- `on_dismiss`はEscキー押下・ダイアログ外クリック(モードレス的操作)・明示的な閉じるボタンいずれからも発火する共通コールバック

| バックエンド | 実装 |
|---|---|
| WinUI3 | `Microsoft::UI::Xaml::Controls::ContentDialog` |
| AppKit | `NSAlert`またはシート(`beginSheet`) |
| GTK4 | `gtk::Dialog` |
| egui / iced | 半透明オーバーレイ上に浮かせた`Window`/`Modal`表現 |

## M.2 `Menu` / `MenuItem`(コンテキストメニュー)

```rust
Menu {
    for item in [
        MenuItem { text: t!("menu-cut"), on_select: cut() },
        MenuItem { text: t!("menu-copy"), on_select: copy() },
        MenuItem { text: t!("menu-paste"), on_select: paste() },
    ] {
        item
    }
}
```

```rust
TextArea {
    text: content
    context_menu: Menu { ... }   // 右クリックで表示するメニューを紐付ける
}
```

## M.3 `Tooltip`

```rust
Button {
    text: t!("notepad-menu-save")
    tooltip: t!("tooltip-save")
    on_click: save_document()
}
```

- `tooltip`は任意のビルトイン要素が持てる共通属性として提供し、ホバー時に各OS標準のツールチップ表示を行う

## M.4 制約の継承

`Dialog`/`Menu`/`Tooltip`はいずれもビルトインであり、内部で`match target::backend()`を持つ。通常の`component`側でこれらを利用する際は、他のビルトイン同様バックエンド分岐を意識する必要はなく、独自部品からこれらを組み合わせて使う場合もG.3の「バックエンド分岐禁止」原則がそのまま適用される(14章ルール15)。

## M.5 まとめ

| 要件 | 対応 |
|---|---|
| モーダルダイアログ | `Dialog { on_dismiss, ... }`、フォーカストラップを自動適用 |
| コンテキストメニュー | `Menu` / `MenuItem`、`context_menu`属性での紐付け |
| ツールチップ | 任意要素が持てる共通属性`tooltip` |
| バックエンドごとの実装差 | ビルトイン内部にのみ分岐を許可し、独自部品からの利用時は分岐禁止原則を維持(14章ルール15) |

---

# 付録N. 描画機能の拡張(Composition相当のビジュアル効果)

付録G.2で定義した`Painter`は塗り・線・テキストの最小セットのみだった。ここでは**WinUI3の`Win2D`(即時描画)と`Composition`(合成レイヤー)に相当する機能一式**を`Painter`の拡張として定義し、グラデーション・シャドウ・ぼかし・変形・アニメーション・レイヤー合成をカバーする。

## N.1 ブラシ(Brush)

塗り潰し・線を「単色」以外でも表現できるよう、`Color`単体ではなく`Brush`型を受け取れるようにする。

```rust
enum Brush {
    Solid(Color),
    LinearGradient { stops: Vec<GradientStop>, start: Point, end: Point },
    RadialGradient { stops: Vec<GradientStop>, center: Point, radius: f32 },
    Image { image: Image, tile: TileMode },
    Acrylic { tint: Color, tint_opacity: f32, blur_amount: f32 },   // WinUI3のAcrylic素材相当
}

struct GradientStop { offset: f32[0.0..=1.0], color: Color }
```

```rust
trait Painter {
    // ...(G.2の基本メソッドに加えて)
    fn fill_rect_brush(&mut self, rect: Rect, brush: &Brush);
    fn stroke_path_brush(&mut self, path: &Path, brush: &Brush, width: f32);
}
```

| Brush種別 | WinUI 3 | AppKit | GTK4 | egui |
|---|---|---|---|---|
| `LinearGradient` | `LinearGradientBrush` | `CGGradient` + `drawLinearGradient` | Cairo `LinearGradient` | `egui::Mesh`によるグラデーション三角形 |
| `Acrylic` | `AcrylicBrush`(ネイティブサポート) | `NSVisualEffectView`重畳で近似 | 非対応(単色フォールバック、17番ルールで警告) | 非対応(単色フォールバック) |

## N.2 図形・パス(Geometry)

ベジエ曲線・弧を含む複雑な形状と、線のスタイル(端点・接合・破線)を定義する。

```rust
struct Path {
    // move_to, line_to, cubic_bezier_to, quadratic_bezier_to, arc_to, close を組み立てて構築
}

struct StrokeStyle {
    cap: LineCap,       // Butt, Round, Square
    join: LineJoin,     // Miter, Round, Bevel
    dash: Vec<f32>,     // 破線パターン(空なら実線)
}
```

```rust
fn draw_meter(p: &mut Painter, level: f64) {
    let mut path = Path::new();
    path.move_to(Point::new(0.0, 20.0));
    path.cubic_bezier_to(Point::new(50.0, 0.0), Point::new(150.0, 40.0), Point::new(200.0, 20.0));
    p.stroke_path_brush(&path, &Brush::Solid(Color::hex("#2ecc71")), 2.0);
}
```

## N.3 エフェクト(Effect)

シャドウ・ぼかし・色調補正など、要素単位で適用する視覚効果。

```rust
enum Effect {
    DropShadow { offset: Point, blur_radius: f32, color: Color },
    Blur { radius: f32 },
    ColorMatrix { matrix: [f32; 20] },   // 彩度・色相調整等
    Opacity { value: f32[0.0..=1.0] },
}
```

```rust
Canvas {
    width: 200
    height: 40
    #[effect(DropShadow { offset: Point::new(0.0, 2.0), blur_radius: 4.0, color: Color::hex("#00000040") })]
    on_paint: draw_meter(painter, level)
}
```

| Effect種別 | WinUI 3 | AppKit | GTK4 | egui |
|---|---|---|---|---|
| `DropShadow` | `Compositor.CreateDropShadow` | `CALayer.shadowOffset/shadowRadius` | Cairo手動合成 | 手動でオフセット矩形を追加描画して近似 |
| `Blur` | `GaussianBlurEffect`(Win2D) | `CIGaussianBlur` | 非対応(17番ルールで警告、フォールバックはブラー無し) | 非対応(同上) |

`#[effect(...)]`が付与された`Canvas`は、内部で`Painter`が返す描画結果をオフスクリーンサーフェスに一度レンダリングしてからエフェクトを適用する(N.5のレイヤー合成の仕組みを利用する)。

## N.4 トランスフォーム(Transform)

```rust
enum Transform {
    Translate(f32, f32),
    Rotate(f32),          // ラジアン
    Scale(f32, f32),
    Skew(f32, f32),
    Matrix([f32; 6]),      // アフィン変換行列
}
```

```rust
trait Painter {
    // ...
    fn push_transform(&mut self, transform: Transform);
    fn pop_transform(&mut self);
}
```

```rust
fn draw_knob(p: &mut Painter, value: f64) {
    p.push_transform(Transform::Rotate((value * std::f64::consts::TAU) as f32));
    p.draw_line(Point::ORIGIN, Point::new(0.0, -20.0), Color::hex("#2ecc71"), 3.0);
    p.pop_transform();
}
```

- `push_transform`/`pop_transform`はスタック方式(SVG/Canvas 2D APIと同じ慣習)とし、ネストした変形を自然に表現できる

## N.5 レイヤー合成(オフスクリーンサーフェス・不透明度・ブレンドモード)

```rust
trait Painter {
    // ...
    fn begin_layer(&mut self, opacity: f32[0.0..=1.0], blend_mode: BlendMode);
    fn end_layer(&mut self);
    fn clip_rect(&mut self, rect: Rect);
    fn clip_path(&mut self, path: &Path);
}

enum BlendMode { Normal, Multiply, Screen, Overlay, Darken, Lighten }
```

- `begin_layer`/`end_layer`は一時的なオフスクリーンサーフェスへの描画を開始・確定する(WinUI3の`Compositor.CreateContainerVisual`相当)。N.3のエフェクトはこの仕組みの上に実装される
- `clip_rect`/`clip_path`は以降の描画を指定領域内にクリップする(スタック方式でネスト可能)

| 機能 | WinUI 3 | AppKit | GTK4 | egui |
|---|---|---|---|---|
| レイヤー合成 | `ContainerVisual` + `CompositionEffectBrush` | `CALayer`の階層合成 | Cairoの`push_group`/`pop_group` | オフスクリーン`egui::Painter`への描画 → テクスチャ合成 |
| クリップ | `Visual.Clip` | `CGContextClip` | `cairo_clip` | `egui::Painter::with_clip_rect` |

## N.6 アニメーション(Transition / KeyframeAnimation)

WinUI3の`Composition`が提供する「暗黙アニメーション(値が変わると自動的に補間される)」と「明示的なキーフレームアニメーション」の両方を用意する。

**暗黙アニメーション(prop変化に自動追従):**

```rust
component ProgressBar {
    #[range(0..=1)]
    #[transition(duration: 200ms, easing: EaseOutCubic)]
    value: f64,
}
```

- `#[transition(duration, easing)]`が付いた`prop`は、値が変化した際にUIが自動的に指定時間・イージング関数で補間描画される。`Canvas`の`on_paint`やビルトイン部品(`ProgressBar`等)の内部実装が、この補間後の中間値を使って再描画する
- `easing`には標準イージング関数(`Linear`, `EaseIn/EaseOut/EaseInOutCubic`, `EaseOutBack`, `Spring { stiffness, damping }`等)を指定する。存在しない名前は14章ルール16により静的エラー

**明示的なキーフレームアニメーション(Canvas内での手続き的制御):**

```rust
Canvas {
    #[animated]
    on_paint: |p| {
        let t = KeyframeAnimation::new()
            .add(0.0, 0.0)
            .add(0.5, 1.2)
            .add(1.0, 1.0)
            .easing(Easing::EaseOutBack)
            .sample(elapsed_time());
        draw_bouncing_icon(p, t);
    }
}
```

- `KeyframeAnimation`は0.0〜1.0の正規化時刻に対する値を複数指定し、`sample(t)`で任意時刻の補間値を取得する。キーフレーム位置が範囲外の場合は14章ルール16によりエラー

## N.7 リッチテキスト描画

```rust
struct TextRun {
    text: String,
    font: Font,
    color: Brush,
    weight: FontWeight,
}

trait Painter {
    // ...
    fn draw_rich_text(&mut self, runs: &[TextRun], layout_rect: Rect, align: TextAlign, wrap: WrapMode);
}
```

- 複数の書式(太字強調・色分け等)が混在するテキストを1回の描画呼び出しでレイアウトできる(WinUI3の`CanvasTextLayout`/AppKitの`NSAttributedString`相当)

## N.8 まとめ

| WinUI3相当の機能 | ElwindUILでの対応 |
|---|---|
| ブラシ(単色/グラデーション/画像/Acrylic) | `Brush` enum + `fill_rect_brush`/`stroke_path_brush` |
| ジオメトリ(ベジエ・弧・線スタイル) | `Path` + `StrokeStyle`(cap/join/dash) |
| エフェクト(シャドウ・ブラー・色調補正) | `Effect` enum + `#[effect(...)]`属性 |
| 変形(移動・回転・拡縮・スキュー) | `Transform` enum + `push_transform`/`pop_transform` |
| Composition(レイヤー合成・クリップ・ブレンド) | `begin_layer`/`end_layer`/`clip_rect`/`clip_path`/`BlendMode` |
| 暗黙アニメーション | `#[transition(duration, easing)]` |
| キーフレームアニメーション | `KeyframeAnimation`(`Canvas`内で手続き的に使用) |
| リッチテキスト | `TextRun` + `draw_rich_text` |

いずれもG.2で定義した`Painter`トレイトの拡張メソッド・付随データ型として`elwindui-core`(付録H)に属し、バックエンドごとの実装差はG.3の原則通り`builtin::Canvas`内部にのみ許可される。GTK4のように一部エフェクト(Acrylic/Blur)が未対応のバックエンドでは、静的警告(14章ルール17)とともに単色/効果無しへのフォールバック描画が行われる。


---

# 付録Q. リスト仮想化

大量データを`for`ループでそのまま描画すると全要素が`Element`として生成され性能が破綻する。表示範囲のみを描画する`VirtualList`ビルトインを提供する。

## Q.1 基本構文

```rust
VirtualList {
    items: documents
    key: |doc| doc.id
    item_height: 32
    render_item: |doc| Row { Text { text: doc.name } }
}
```

- `items` — 全データ(`Vec<T>`)
- `key` — 要素の同一性を判定する関数。リスト順序が変わっても同じ`key`を持つデータは`Element`インスタンスを使い回す(Reactのkey付きリコンサイルと同じ考え方)
- `item_height` — 固定高さの場合はこの値でMeasureパス(付録H.2)をスキップし、表示範囲を定数時間で計算する
- `render_item` — 1件分の`view`を返すクロージャ

## Q.2 可変高さの場合

```rust
VirtualList {
    items: documents
    key: |doc| doc.id
    estimated_item_height: 40
    render_item: |doc| DocumentCard { doc }
}
```

- `estimated_item_height`のみを指定した場合、実際の高さは初回描画時に`LayoutNode::measure`(付録H.2)で計測し、以後はキャッシュして再利用する

## Q.3 要素の再利用(リサイクル)

- スクロールに応じて画面外に出た`Element`インスタンスはすぐに破棄せず、プールに戻して次に表示範囲へ入るデータの描画に再利用する
- 再利用されるインスタンスでは`on_mount`(付録I)は初回プール生成時のみ発火し、以降は`prop`の更新のみが行われる(通常の差分更新、4章)。これによりライフサイクルフックの発火回数を抑えつつ、GUI側の状態(スクロール位置、フォーカス等)を不要に破棄しない

## Q.4 `key`未指定時の挙動

`key`を指定せずに`items`の順序が変わる更新を行うと、挿入位置ベースの再利用にフォールバックし、意図しない要素の使い回し(例:別データなのに同じ`Element`インスタンスが再利用されフォーカス状態が誤って引き継がれる)が起きうる。これを防ぐため、14章ルール23により静的警告を出す。

## Q.5 バックエンド対応

| バックエンド | 実装 |
|---|---|
| WinUI3 | `ItemsRepeater` + `VirtualizingLayout` |
| AppKit | `NSTableView` / `NSCollectionView`(セル再利用機構をそのまま利用) |
| GTK4 | `gtk::ListView` + `GListModel`(GTK4は元々仮想化前提の設計) |
| egui / iced | `elwindui-core`の`LayoutEngine`(付録H.2)がビューポート情報を持ち、表示範囲外の`render_item`呼び出し自体をスキップする |

## Q.6 まとめ

| 要件 | 対応 |
|---|---|
| 大量データの効率描画 | `VirtualList { items, item_height/estimated_item_height, render_item }` |
| 順序変更時の安全な再利用 | `key`関数による同一性判定 |
| リサイクルとライフサイクルの整合 | プール再利用時は`on_mount`を再発火させず、prop更新のみで反映 |
| `key`未指定時の注意喚起 | 14章ルール23による静的警告 |


---

# 付録T. クリップボード・ドラッグ&ドロップ・ファイルダイアログ

OS機能へのアクセスを、GUI要素ではなく`platform::`名前空間の関数として提供する(9章の`env::*`、5章で触れた`external::*`と同じ「明示的な入口」の考え方)。

## T.1 クリップボード

```rust
platform::clipboard::write_text(&content);
let text: Option<String> = platform::clipboard::read_text();
```

## T.2 ファイルダイアログ(非同期)

```rust
#[command(async)]
open: Command = command!(async || {
    if let Some(path) = platform::file_dialog::open(FileFilter::new(t!("filter-text"), &["txt"])).await {
        content = fs::read_to_string(&path).await.unwrap_or_default();
    }
}),
```

- ファイルダイアログは本質的に非同期(ユーザーの操作待ち)であるため、常に`Future`を返し、付録Pの`#[command(async)]`パターンと組み合わせて使う

## T.3 ドラッグ&ドロップ

```rust
TextArea {
    text: content
    draggable: false
    on_drop: |files: Vec<PathBuf>| open_files(files)
}
```

- `on_drag_start` / `on_drop` / `draggable: bool` は任意のビルトイン要素が持てる共通属性として提供する(付録Mの`tooltip`/`context_menu`と同じ位置づけ)

## T.4 バックエンド対応

| 機能 | WinUI3 | AppKit | GTK4 | egui / iced |
|---|---|---|---|---|
| クリップボード | `Clipboard`/`DataPackage` | `NSPasteboard` | `Gdk::Clipboard` | `arboard`クレート経由 |
| ファイルダイアログ | `FileOpenPicker`/`FileSavePicker` | `NSOpenPanel`/`NSSavePanel` | `gtk::FileChooserNative` | `rfd`クレート経由 |
| ドラッグ&ドロップ | `DragDrop`イベント | `NSDraggingDestination` | `Gtk::DropTarget` | Canvas内ヒットテストで独自実装 |

## T.5 まとめ

| 要件 | 対応 |
|---|---|
| クリップボード操作 | `platform::clipboard::read_text()` / `write_text()` |
| ファイルダイアログ | `platform::file_dialog::open()`/`save()`(非同期、付録Pと連携) |
| ドラッグ&ドロップ | `draggable` / `on_drag_start` / `on_drop` 共通属性 |
| OS機能アクセスの一貫性 | `platform::`名前空間に集約し、`env::`/`external::`と同じ「明示的な入口」の思想を踏襲 |

---

# 付録X. `MenuBar` / `MenuBarItem`(アプリケーションメインメニュー)

付録Mの`Menu`/`MenuItem`は右クリック等で浮くコンテキストメニューだった。ここではOS標準の「画面最上部の固定メニューバー」(macOSのメニューバー、WinUI3/GTK4のウィンドウ内メニュー相当)に対応する`MenuBar`/`MenuBarItem`を定義する。

## X.1 基本構文

```rust
view NotepadWindow {
    Window {
        title: vm.window_title
        menu_bar: MenuBar {
            MenuBarItem {
                text: t!("menu-file")
                Menu {
                    MenuItem { text: t!("menu-new"), #[shortcut(winui3: "Ctrl+N", appkit: "Cmd+N")], on_select: vm.new_tab.execute() }
                    MenuItem { text: t!("menu-open"), #[shortcut(winui3: "Ctrl+O", appkit: "Cmd+O")], on_select: vm.open.execute() }
                    MenuItem { text: t!("menu-save"), #[shortcut(winui3: "Ctrl+S", appkit: "Cmd+S")], on_select: vm.save.execute(), enabled: vm.save.can_execute }
                }
            }
            MenuBarItem {
                text: t!("menu-edit")
                Menu {
                    MenuItem { text: t!("menu-undo"), #[shortcut("Ctrl+Z")], on_select: vm.undo.execute() }
                    MenuItem { text: t!("menu-redo"), #[shortcut(winui3: "Ctrl+Y", appkit: "Cmd+Shift+Z")], on_select: vm.redo.execute() }
                }
            }
        }

        // ... 既存のTabView等
    }
}
```

- `menu_bar`は`Window`が持てる任意属性で、`MenuBar { MenuBarItem { ... } ... }`を渡す
- `MenuBarItem`は最上段(File/Editのようなドロップダウンの見出し)であり、中身は付録Mの`Menu`/`MenuItem`をそのまま再利用する。新しい項目プリミティブは導入しない
- `MenuItem`は付録Kの`#[shortcut(...)]`を追加で持てる。表示されるアクセラレータ文字列はOSごとの標準表記(macOSは⌘、WinUI3/GTK4はCtrl+)に自動変換される(付録K.2と同じ規則)
- `enabled`は`Button`(付録F)と同じ共通属性で、`vm.save.can_execute`のような`Command`のcan_execute式をそのまま束縛できる

## X.2 バックエンド対応

| バックエンド | 実装 | 状態 |
|---|---|---|
| AppKit | `NSMenu`ツリーを構築し`NSApplication.mainMenu`に設定。`MenuItem`ごとに`NSMenuItem` + target/action | 実装済み |
| WinUI3 | `Microsoft::UI::Xaml::Controls::MenuBar` / `MenuFlyoutItem` | 未実装(仕様のみ。他バックエンドスタブと同じ方針) |
| GTK4 | `gtk::PopoverMenuBar` + `gio::Menu` | 未実装 |
| egui / iced | ウィンドウ内メニュー行相当のウィジェットで代替 | 未実装 |

## X.3 まとめ

| 要件 | 対応 |
|---|---|
| アプリ最上部の固定メニュー | `Window { menu_bar: MenuBar { MenuBarItem { ... } } }` |
| ドロップダウンの中身 | 付録Mの`Menu`/`MenuItem`を再利用(新規プリミティブなし) |
| キーボードアクセラレータ表示 | `MenuItem`が付録Kの`#[shortcut(...)]`を追加で持てる |
| 有効/無効の切り替え | `MenuItem.enabled`(`Button`と同じ共通属性) |
| バックエンド実装状況 | AppKitのみ実装、他は仕様上のマッピングのみ(他backendスタブと同じ方針) |

---

# 付録Y. `TabView` / `TabItem`(複数ドキュメントタブ)

複数のドキュメント(ファイル)を1つのウィンドウ内でタブ切り替えして扱うためのビルトイン。付録Qの`VirtualList`と同じ「データ配列 + 1件分の描画クロージャ」という形を踏襲するが、対象はせいぜい数十件程度の小規模なリストであり、`VirtualList`のような仮想化・再利用プールは持たない(選択中の1件を除き非表示のタブも実体は保持される)。

## Y.1 基本構文

```rust
view NotepadWindow {
    Window {
        title: vm.window_title
        menu_bar: MenuBar { /* 付録X */ }

        TabView {
            tabs: vm.documents
            key: |doc| doc.id
            render_label: |doc| doc.file_name
            render_content: |doc| TextArea { text: doc.content }
            selected: vm.active_tab
            on_select: |index| vm.active_tab = index
            on_close: |index| vm.close_tab_execute(index)
            on_new_tab: vm.new_tab.execute()
            closable: true
        }
    }
}
```

- `tabs` — 表示するデータの配列(`Vec<T>`型の`#[observable]`な`viewmodel`フィールドを想定)
- `key` — 付録Qと同じ同一性判定関数。タブの並び替え・増減があっても同じ`key`のデータは同じウィジェットインスタンスを使い回す
- `render_label` — タブ見出しに表示する文字列を返すクロージャ
- `render_content` — 選択中タブの中身として描画する`view`を返すクロージャ(常に選択中の1件のみが表示される)
- `selected` — 現在選択中のインデックス(`usize`の観測可能値)。タブクリックで内部的に更新され`on_select`が発火する
- `closable` / `on_close` — タブごとの閉じるボタン("×")の表示可否と押下時のコールバック
- `on_new_tab` — タブ列末尾の"+"ボタン押下時のコールバック

## Y.2 実装範囲の注記(重要)

付録Qの`VirtualList`は「任意個数の要素を仮想化して効率描画する」汎用リスト機構だが、`.elwind`コンパイラは現時点で「任意の`view`を実行時に動的な個数だけ生成する」という汎用機能自体を持たない(5章`for`ループの実行時版に相当する機能はまだない)。`TabView`はこの汎用機構を土台にするのではなく、**タブ切り替えUIに特化した専用のコード生成パス**として実装する — タブの追加・削除に応じてウィジェットを個別に生成・破棄するロジックを`elwindui-codegen`が`TabView`向けに直接持つ形であり、任意の`view`を対象にした汎用的な動的リスト生成(`VirtualList`の完全な実装や`for`ループの実行時版)は引き続き将来の課題として残る。これは付録Pの`Dispatcher`/`spawn`が仕様上定義されつつ実行時機構としては未実装であるのと同じ位置づけの、意図的なスコープの区切りである。

## Y.3 バックエンド対応

| バックエンド | 実装 | 状態 |
|---|---|---|
| AppKit | `NSStackView`によるタブ見出し行(タイトル + 閉じるボタン + 末尾の"+"ボタン)、選択に応じてコンテンツ領域を差し替え | 実装済み |
| WinUI3 | `Microsoft::UI::Xaml::Controls::TabView` | 未実装 |
| GTK4 | `gtk::Notebook` | 未実装 |
| egui / iced | 内部状態によるタブ見出し行の描画 + 選択中コンテンツの切り替え | 未実装 |

## Y.4 まとめ

| 要件 | 対応 |
|---|---|
| 複数ドキュメントの保持 | `tabs: <Vec<T>型の観測可能フィールド>` |
| タブ見出し・内容の描画 | `render_label` / `render_content`クロージャ |
| タブ切り替え | `selected` + `on_select` |
| タブを閉じる | `closable` + `on_close` |
| 新規タブ | `on_new_tab`("+"ボタン) |
| 実装範囲 | タブ専用に特化した動的ウィジェット生成パス。任意`view`向けの汎用動的リスト生成は未実装のまま(付録Q参照) |

