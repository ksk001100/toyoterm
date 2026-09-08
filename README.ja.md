# toyoterm

<p align="center">
  <img src="packaging/app-icon.png" alt="toyoterm icon" width="180">
</p>

[English](README.md)

toyotermはRustと組み込みmrubyによる、プログラム可能な実験的ターミナルエミュレータです。
端末処理はネイティブで行い、Rubyを設定、キーバインド、イベント、コマンドに使用します。
自分で使うために作っている個人の実験的プロジェクトです。

> [!IMPORTANT]
> 開発中です。Workspace・Tab・分割Paneは独立したPTYと端末セッションを持ちます。複数OSウィンドウは初回リリース後の対応予定です。

## 機能

- ネイティブPTY、`alacritty_terminal`によるVT解析、`wgpu`と`glyphon`によるGPU描画
- Workspace、Tab、Pane分割・ズームと独立したシェルセッション
- UTF-8・IME入力、入力時に最下部へ戻るスクロールバック、検索、選択、クリップボードへのコピー・貼り付け
- OSC 8リンク、URL自動検出、作業ディレクトリとコマンド状態を通知するシェル連携
- Sixel・Kitty画像プロトコル・iTerm2 OSC 1337による画像表示（[対応範囲と実行例](docs/image-protocols.md)）
- mruby 4.0による設定、ネイティブ・Rubyキーバインド、イベント、コマンド、プラグイン、テーマ
- アトミックな設定リロード、ライブRubyコンソール、ローカルIPC、ウィンドウバーと壁紙

## 現在の状態

主な開発環境はLinuxです。CIではLinux・macOS・Windowsのビルド、テスト、パッケージ作成、
GUI起動スモークテストを実行します。実機での対話的な検証は別途必要です。
詳細は[プラットフォーム検証](docs/platform-validation.md)を参照してください。
複数OSウィンドウ、セッション永続化は初回リリースの対象外です。
Rubyの`Window`は、単一OSウィンドウ内で表示するMux上のウィンドウを表します。

## ビルドと起動

最近の安定版Rust、同梱mrubyをビルドするCコンパイラ、`winit`・`wgpu`に必要な
プラットフォームライブラリが必要です。LinuxではWaylandまたはX11セッションと、
xkbcommon・`pkg-config`などの開発用ライブラリを用意してください。Windowsの描画にはDirectX 12が必要です。

```sh
cargo run --locked
```

最適化ビルドは`cargo build --release --locked`で作成し、`target/release/toyoterm`
（Windowsでは`target/release/toyoterm.exe`）を実行します。

リリース成果物は、Linuxでは`install.sh`付きアーカイブ、macOSではDMGまたはappバンドルの
アーカイブ、Windowsでは任意のユーザー単位インストーラ付きportable zipです。
[インストール・更新・削除・チェックサム](docs/packaging.md)を参照してください。

## 設定

組み込みGUIキーバインドはありません。設定ファイルを保存して次のように起動します。

```sh
toyoterm --config /path/to/config.rb
```

```ruby
Toyoterm.configure do |config|
  config.font.family = "monospace"
  config.font.size = 14

  config.keys do
    ctrl_shift("t").new_tab
    ctrl_shift("e").split(:right)
    ctrl_shift("r").reload_config
    primary_shift("c").copy_selection
    primary_shift("v").paste_clipboard
  end
end
```

[minimal_config.rb](examples/minimal_config.rb)または[default_config.rb](examples/default_config.rb)
を出発点にできます。プロンプトのアイコン表示にはNerd Fontが便利です。インストール済みの正確なファミリー名を指定してください。

設定は`--config`、`TOYOTERM_CONFIG_FILE`、プラットフォーム既定パスの順に選択します。
Linux・macOS・Windowsのパスとエラー時の復旧は[設定の読込](docs/mruby-api.md#loading-configuration)
を参照してください。`toyoterm reload`で再読込し、`toyoterm ruby console`でRubyによるライブ変更ができます。

設定とプラグインは信頼済みコードとして動作し、ファイル・プロセス・環境変数・クリップボードへ
アクセスできます。サンドボックスではありません。組み込みランタイムはmrubyのため、
CRubyの全標準ライブラリやgemは利用できません。組み込みランタイムでは、mrubyの
ポータブルな標準ライブラリ、数値、メタプログラミングAPIを利用できます。OS依存の
I/O・socket gemは含めず、toyotermのホストAPIを使用します。詳細はAPIリファレンスを
参照してください。

## 操作とドキュメント

- [利用ガイド](docs/usage.md)：マウス操作、CLI、ログ、トラブルシューティング
- [mruby APIリファレンス](docs/mruby-api.md)：設定、キーバインド、コールバック、プラグイン、テーマ
- [シェル連携](docs/shell-integration.md)：作業ディレクトリとコマンド状態の通知
- [ドキュメント一覧](docs/README.md)：利用者・開発者向けガイド

通常のキー入力はシェルへ送信し、Tab・Workspaceのクリックで切り替え、ドラッグでテキストを
選択します。Control+クリック（macOSはCommand+クリック）で許可されたWeb・メールリンクを開きます。
最後のPaneが終了するとアプリも終了します。

## 開発

locked指定の検証コマンドとネイティブスモークテストは[開発ガイド](docs/development.md)、
パッケージ作成は[リリースチェックリスト](docs/releasing.md)を参照してください。
[クレート構成](docs/architecture.md)と[スレッド契約](docs/threading.md)で、ネイティブ側の所有権と
専用スクリプトスレッドを説明しています。静的キーバインドはRubyを呼び出さず、
Rubyコールバックの返すコマンドはメインスレッドで適用します。

## ライセンス

toyotermは[MIT License](LICENSE)で配布します。

リポジトリには公式mruby 4.0.0のamalgamationをMITライセンスのもとで同梱しています。詳細は[サードパーティー通知](THIRD_PARTY_NOTICES.md)と[保存しているmrubyのライセンス](vendor/mruby/LICENSE)を参照してください。Rust依存ライブラリのライセンスはCIで`cargo-deny`を使って検査します。
