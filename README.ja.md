# toyoterm

[English](README.md)

toyotermは、Rustと組み込みmrubyで作る実験的なプログラマブル・ターミナルエミュレータです。ターミナルのホットパスはネイティブ実装のまま保ち、設定、動的キーバインド、ランタイムイベント、コマンドにRubyを利用します。

> [!IMPORTANT]
> toyotermは活発に開発中です。GUIの各Workspace、タブ、分割Paneは独立したPTYとターミナルセッションを持ちます。複数OSウィンドウ対応は初期リリース後へ明示的に延期しています。

## 機能

- ネイティブPTYとプラットフォーム標準シェル
- `wgpu`と`glyphon`によるGPU描画ウィンドウ
- `alacritty_terminal`を利用したVTシーケンス解析
- UTF-8入力、リサイズ、スクロールバック、マウスホイール
- IME preedit描画とcommit・cancel処理
- テキスト選択とクリップボードのコピー・ペースト
- mruby 4.0を組み込んだ設定ランタイム
- ネイティブコマンドを発行できる動的Rubyキーバインド
- アトミックな設定リロード。不正な更新時は以前の設定を維持
- `app_started`と`config_reloaded`のRubyイベント
- タブ、ペイン分割、ワークスペースに対応したネイティブCommand・Muxモデル
- PaneごとにPTYとTerminalBackendを持つGUIタブ
- Paneごとのresizeとfocusに対応した分割Pane描画
- マウス操作とキーボード操作に対応したタブバー
- Workspaceごとのfocus復元に対応したWorkspaceバー

## 現在の状態

主な開発環境はLinuxです。アーキテクチャと依存ライブラリはクロスプラットフォームを意識していますが、macOSとWindowsではまだ十分な動作検証を行っていません。

GUIへ未接続の機能：

- 複数OSウィンドウ
- ライブRuby REPLとリモート操作CLI
- 検索、リンク、画像プロトコル、セッション永続化

## ビルドと起動

### 必要なもの

- 新しい安定版Rustツールチェーン
- 同梱mrubyをビルドするためのCコンパイラ
- `winit`・`wgpu`が要求する各プラットフォームの開発ライブラリ

LinuxではWaylandまたはX11のデスクトップセッションが必要です。不足している場合は、利用中のディストリビューションからCビルドツール、`pkg-config`、Wayland/X11、xkbcommonの開発パッケージをインストールしてください。

リポジトリをcloneした後、次のコマンドで起動します。

```sh
cd toyoterm
cargo run
```

最適化したバイナリをビルドして起動する場合：

```sh
cargo build --release --locked
./target/release/toyoterm
```

設定ファイルを明示する場合：

```sh
cargo run -- --config /path/to/config.rb
```

## 設定

設定ファイルは次の優先順位で読み込まれます。

1. `--config`で指定したパス
2. `TOYOTERM_CONFIG_FILE`
3. `~/.config/toyoterm/config.rb`

デフォルトパスのファイルは省略可能です。明示的に指定したファイルは存在し、正しいRubyである必要があります。

設定例：

```ruby
Toyoterm.configure do |config|
  config.font do |font|
    font.family = "monospace"
    font.size = 14
    font.weight = 400
  end

  config.colors do |colors|
    colors.background = "#090b0e"
    colors.foreground = "#dce1e8"
    colors.cursor = "#f5f7fa"
    colors.selection = "#375891"
  end

  config.window.opacity = 0.96
  config.scrollback_lines = 20_000
  config.leader key: "b", mods: "CTRL", timeout: 1000

  # 必要な場合はシェルを明示できます。省略時はプラットフォーム標準です。
  # config.default_shell = "/bin/zsh"

  config.bind "CTRL+SHIFT+H" do |context|
    context.pane.send_text("echo hello from mruby\n")
  end

  config.bind "CTRL+SHIFT+R" do
    Toyoterm.reload_config
  end

  # 一般的な操作はNative Actionへcompileされ、キー入力時にmrubyを呼びません。
  config.keys do
    leader("v").split(:right)
    ctrl_shift("e").split(:right)
    ctrl_shift("o").activate_pane(:right)
    ctrl_shift("t").new_tab
  end
end

Toyoterm.on :app_started do |event|
  event.pane.send_text("echo toyoterm started\n")
end

Toyoterm.on :config_reloaded do |event|
  event.pane.send_text("echo config reloaded\n")
end
```

### キーバインド

キー名は大文字・小文字を区別しません。修飾キーには`CTRL`、`SHIFT`、`ALT`、`SUPER`などを使用します。名前付きキーは`ENTER`、`TAB`、`SPACE`、矢印キー、ナビゲーションキー、`F1`から`F12`に対応しています。

`config.keys`では`ctrl`、`ctrl_shift`、`alt`、`super_key`、`leader`、`physical`ヘルパーを使用できます。`physical("KeyH", "CTRL")`のように指定すると、論理文字ではなく物理キー位置へ割り当てられます。両方が一致した場合はphysical設定、組み込みGUIショートカットと競合した場合はユーザー設定を優先します。同じchordの重複定義は設定エラーです。

`config.leader`では、ミリ秒単位のtimeout付きLeader prefixをネイティブ側へ設定できます。`leader("v")`の割り当てはmrubyを呼ばずに解決されます。Leader prefix自体は破棄し、不一致またはtimeout後の次キーは通常のキー処理へ戻します。repeat、IME入力、フォーカス喪失、設定reloadではLeader待機状態を解除します。

割り当てのないキーはmrubyを呼ばず、ネイティブのターミナルキーエンコーダへ直接渡されます。Ruby callbackで例外が発生した場合はエラーをログへ出し、シェルの実行を継続します。

### ホットリロード

`Toyoterm.reload_config`は、起動時に選択されたものと同じファイルを再読込します。新しいソースは別のmruby VMで評価・検証され、成功した場合だけ有効な設定と入れ替わります。正常に再読込できると、実行中のターミナルセッションを維持したまま、配色、フォントメトリクス、透明度、スクロールバック、キーバインド、イベントハンドラを更新します。

`default_shell`を変更しても実行中のシェルは置き換えません。新しいターミナルセッションを作成するときに適用されます。

実行可能な最小設定例は`examples/minimal_config.rb`にあります。`toyoterm --config examples/minimal_config.rb`で試せます。

組み込みランタイムはCRubyではなくmrubyです。toyotermが明示的にbundleしていないCRuby gem、native extension、完全なCRuby標準ライブラリは利用できません。現在の設定・イベントAPIでは不要なため、v0.1では`mruby-time`をbundleしません。

## 操作

- 通常のキー入力：PTYへ入力を送信
- Linux・Windowsの`Ctrl+Shift+C`またはmacOSの`Cmd+C`：選択範囲をコピー
- Linux・Windowsの`Ctrl+Shift+V`またはmacOSの`Cmd+V`：貼り付け
- `Ctrl+Shift+T`：新しいタブを作成
- `Ctrl+Shift+W`：active Tabを閉じる（最後のタブは維持）
- `Ctrl+Tab` / `Ctrl+Shift+Tab`：次／前のタブをactivate
- `Ctrl+Shift+\` / `Ctrl+Shift+-`：active Paneを右／下へ分割
- `Ctrl+Shift+矢印`：指定方向の最寄りPaneへfocus
- `Ctrl+Shift+Q`：active Paneを閉じる（最後のPaneは維持）
- `Ctrl+Shift+N`：Workspaceを作成してactivate
- `Ctrl+Alt+Left` / `Ctrl+Alt+Right`：前／次のWorkspaceをactivate
- Workspaceまたはタブのラベルをクリック：対象をactivate
- 左マウスボタンでドラッグ：テキストを選択
- マウスホイール：履歴をスクロール。アプリケーションがマウスレポートを要求している場合はホイール入力を送信

## CLI

```text
toyoterm [--config PATH]
toyoterm gui [--config PATH]
toyoterm list
toyoterm demo
toyoterm pty-demo
toyoterm screen-demo
toyoterm version
toyoterm help
```

現在の`list`と`demo`はメモリ上のMuxモデルを確認するためのコマンドです。起動済みGUIの状態確認や操作は行いません。

## セキュリティ

設定ファイルは、組み込みmrubyランタイムで信頼済みのRubyコードとして評価されます。現在のtoyotermは設定や将来のプラグインに対するサンドボックス、capability制限を提供していません。信頼できる入手元の設定だけを読み込んでください。

## 開発

テストと静的検査：

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
cargo fmt --check
sh scripts/check-licenses.sh
```

`dist/`以下にリリースアーカイブを作成：

```sh
sh scripts/package.sh
```

アーカイブにはtoyoterm本体、プロジェクトのライセンス、サードパーティー通知、mrubyのライセンスが含まれます。

## アーキテクチャ

```text
winit events
    ├─ native key binding resolver ─> mruby callback ─> native Command
    └─ terminal key encoder
                                      ↓
                                  native Mux
                                      ↓
                                     PTY
                                      ↓
                              alacritty_terminal
                                      ↓
                                wgpu + glyphon
```

組み込みmruby VMは単一スレッドで動作します。Ruby callbackはターミナルやMuxの内部状態を直接変更せず、ネイティブコマンドをキューへ追加します。

## ライセンス

toyotermは[MIT License](LICENSE)で配布します。

リポジトリには公式mruby 4.0.0のamalgamationをMITライセンスのもとで同梱しています。詳細は[サードパーティー通知](THIRD_PARTY_NOTICES.md)と[保存しているmrubyのライセンス](vendor/mruby/LICENSE)を参照してください。Rust依存ライブラリのライセンスはCIで`cargo-deny`を使って検査します。
