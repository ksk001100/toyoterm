# toyoterm

<p align="center">
  <img src="packaging/app-icon.png" alt="toyoterm icon" width="180">
</p>

[English](README.md)

toyotermはRustと組み込みSteelによる、プログラム可能な実験的ターミナルエミュレータです。
端末処理はネイティブで行い、Schemeを設定、キーバインド、イベント、コマンドに使用します。
自分で使うために作っている個人の実験的プロジェクトです。

> [!IMPORTANT]
> 開発中です。Workspace・Tab・分割Paneは独立したPTYと端末セッションを持ちます。OSウィンドウはGUIプロセスごとに1つです。`toyoterm`を再度実行すると、別プロセスと別ウィンドウが起動します。1プロセス内の複数OSウィンドウは初回リリース後の対応予定です。

## 機能

- ネイティブPTY、`alacritty_terminal`によるVT解析、`wgpu`と`glyphon`によるGPU描画
- Workspace、Tab、Pane分割・ズームと独立したシェルセッション
- 全角文字上ではブロック型・下線型カーソルを2セル幅で表示。ブロック型カーソルに重なる文字は、カーソル文字色の明示指定がなければ、読みやすい色を自動選択して表示
- UTF-8・IME入力、Kitty keyboard protocolの段階的拡張、マウストラッキングとSGRマウスレポーティング、長さ制限付きOSC 50実行時フォントファミリー切替、Kitty OSC 66のセルをまたぐ文字と分数倍率表示、入力時に最下部へ戻るスクロールバック、検索、選択、クリップボードへのコピー・貼り付け
- 独立したアイコンタイトル情報を含むOSCタイトル・iTerm2セッション色／ANSI色／リンク色／選択色／カーソル文字色／下線色／ビジュアルベル色／透明背景色／Display P3色／登録済みSchemeテーマプリセットを含むパレット／動的色／文字属性特殊色の制御・色スタック、OSC 8リンク、タブ色・サイズ制限付きPaneバッジ・iTerm2の全進捗状態・セッション状態インジケーター、明示許可・サイズ制限付きクリップボードコピー・URL起動・フォーカス要求、標準サウンド・名前付きアイコン・転送アイコンのキャッシュと表示条件・優先度・全デスクトップの応答通知に対応したデスクトップ通知、カーソル花火を含む注意要求、URL自動検出、作業ディレクトリ・リモートホスト・連携バージョン・終了状態付きコマンドマーカーとキャプチャ出力消去・プロンプト移動・iTerm2マーク移動を行うシェル連携（[対応表](docs/osc-support.md)）
- Linux・macOS・Windowsでの7/8ビットSixel・Kitty画像プロトコル（TUIフレームワークが使うUnicode placeholderを含む）・iTerm2 OSC 1337による画像表示と、明示許可・サイズ制限付きiTerm2／Kittyファイルダウンロードおよび固定ルート内に限定したKittyアップロード。子セッションでは外側の端末から継承した古い機能判定用環境変数を除去し、TUI画像ライブラリがtoyotermを正しく判定できるようにします（[対応範囲と実行例](docs/image-protocols.md)）
- Steel 0.8.3による設定、ネイティブ・Schemeキーバインド、イベント、コマンド、ローカルSchemeライブラリ、テーマ、検索可能な選択オーバーレイ
- アトミックな設定リロード、ライブSchemeコンソール、ローカルIPC、ウィンドウバーと壁紙（明示許可・固定ルート内に限定したOSC壁紙変更を含む）
- OSCの許可設定を `osc-set!` に集約。ダウンロード・アップロード・OSC壁紙変更は絶対パスのディレクトリを指定するだけで有効化できます（[設定API](docs/steel-api.md#settings)）

## 現在の状態

主な開発環境はLinuxです。CIではLinux・macOS・Windowsのビルド、テスト、パッケージ作成、
GUI起動スモークテストを実行します。各OSの実機でも主要な動作を一通り確認しています。
リリース候補版では[プラットフォーム検証](docs/platform-validation.md)の項目を改めて確認します。
CIとライセンス検査はPRと`master`へのpushで実行します。作業ブランチへのpushでは実行しません。
1つのGUIプロセス内での複数OSウィンドウとセッション永続化は初回リリースの対象外です。
Schemeのwindowは、そのGUIプロセスの単一OSウィンドウ内で表示するMux上のウィンドウを表します。

## ビルドと起動

最近の安定版Rust、`winit`・`wgpu`に必要な
プラットフォームライブラリが必要です。LinuxではWaylandまたはX11セッションと、
xkbcommon・`pkg-config`などの開発用ライブラリを用意してください。Windowsの描画にはDirectX 12が必要です。

```sh
cargo run --locked
```

最適化ビルドは`cargo build --release --locked`で作成し、Linux・macOSでは
`target/release/toyoterm`を実行します。Windowsではconsoleを表示しないGUI起動に
`target/release/toyoterm-gui.exe`、CLIコマンドに`target/release/toyoterm.exe`を使用します。

リリース成果物は、Linuxでは`install.sh`付きアーカイブ、macOSではDMGまたはappバンドルの
アーカイブ、Windowsではユーザー単位のMSIインストーラーとportable zipです。
Windowsのスタートメニューから起動すると、作業ディレクトリはユーザーのホームになります。
Windows MSIのメタデータにはCargoバージョンの数値部分を使用します（例：`0.2.0-dev` → `0.2.0`）。
実行ファイルと配布ファイル名には完全なバージョンを保持します。
[インストール・更新・削除・チェックサム](docs/packaging.md)を参照してください。
配布パッケージには、リンク先のガイドと実行可能なサンプルも含まれます。
macOSアプリは未署名のため、初回起動時に[インストール手順](docs/packaging.md#macos)に従って
手動で許可する必要がある場合があります。「壊れているため開けません」と表示される場合の
対処として、DMGからインストールしたアプリのチェックサムを確認し、配布物を信頼できる場合は
ターミナルで次を実行してから再度起動してください。

```sh
xattr -dr com.apple.quarantine /Applications/toyoterm.app
```

## 設定

Scheme設定を保存し、`toyoterm --config /path/to/config.scm`で起動します。
組み込みGUIキーバインドはありません。

```scheme
(config-set! 'font.family "monospace")
(config-set! 'font.size 14)
(bind-action! "CTRL+SHIFT+T" 'new-tab)
(bind-action! "CTRL+SHIFT+E" 'split 'right)
(bind-action! "CTRL+SHIFT+R" 'reload-config)
```

[minimal_config.scm](examples/minimal_config.scm)または
[default_config.scm](examples/default_config.scm)を出発点にできます。
設定は`--config`、`TOYOTERM_CONFIG_FILE`、プラットフォーム既定の`config.scm`の順に選択します。
設定値と検証規則は[Steel APIリファレンス](docs/steel-api.md)を参照してください。

`toyoterm reload`でアトミックに再読込します。`toyoterm steel console`では、
複数行の定義と入力間で保持する変数を使ってSchemeを評価できます。
Windowsのコンソールモードは終了時に復元します。ペイン内のCLIは、
GUIが複数起動している場合も、そのペインのGUIへ接続します。

設定とSchemeライブラリは信頼済みのローカルコードであり、プロセスと同じ権限で
ファイル・プロセス・環境変数・クリップボードへアクセスできます。サンドボックスではありません。
Steelの関数、レキシカルクロージャ、マクロ、コレクション、モジュール、標準ファイルAPIを利用できます。
`(require "lib.scm")`でライブラリを明示的に読み込みます。相対importは宣言元のソースを基準に解決します。
Rubyソースの実行互換はありません。

`command!`・`on!`・`bind!`は解除可能なコールバックを登録し、`bind-action!`はネイティブで実行します。
コールバックは不変スナップショットを受け取り、型付きネイティブコマンドを返します。
`context-action!`は呼出元の対象を維持します。`spawn`は外部コマンドの結果を同期取得し、
`async!`はスクリプト・GUIスレッドの外で実行します。完了結果はタスクハンドルに保持し、
完了済みタスクのグローバルレジストリを蓄積しません。
`bar!`・`widget`・`async-widget`で個別の更新間隔を持つバーを定義します。
非同期ウィジェットの作業ディレクトリは、コンテキストを受け取る関数で現在のペインに追従できます。
空のウィジェット文字列は区切り文字で連結する対象から除きます。
テーマ、選択UI、OSC許可、壁紙、ライブ設定も[Steel API](docs/steel-api.md)から利用できます。

## 操作とドキュメント

- [利用ガイド](docs/usage.md)：マウス操作、CLI、ログ、トラブルシューティング
- [Steel APIリファレンス](docs/steel-api.md)：設定、キーバインド、コールバック、Schemeライブラリ、テーマ
- [シェル連携](docs/shell-integration.md)：作業ディレクトリ・リモートホスト・コマンド状態の通知とプロンプト移動
- [ドキュメント一覧](docs/README.md)：利用者・開発者向けガイド

通常のキー入力はシェルへ送信し、Tab・Workspaceのクリックで切り替え、ドラッグでテキストを
選択します。Control+クリック（macOSはCommand+クリック）で許可されたWeb・メールリンクを開きます。
最後のPaneが終了するとアプリも終了します。

## 開発

[端末fuzzingガイド](docs/fuzzing.md)に、6種類のcargo-fuzz target、通常テストでのseed再実行、
CI smoke、各1時間のcampaign、crash regressionの運用手順をまとめています。

任意実行の[end-to-end性能baseline](docs/performance-baseline.md)で、実際のPTYからframeまで、
pane数、画像、検索、Scheme overhead、config reloadを計測できます。commit比較用のJSON結果を保存し、
通常CIには時間thresholdを設定しません。
端末間の出力スループット比較には、各端末内で `scripts/terminal-throughput.py` を実行します。
同ガイドにDSR応答による指標と測定範囲の制限を記載しています。
通常の独立した文字はpaneごとに最大512件の整形済みbufferを共有し、CJKや絵文字のfont fallbackの
重複処理を減らします。グリッド上の位置と選択色は維持します。

短時間のメモリ soak は `python3 scripts/soak-test.py headless --output .soak-results/short` で任意実行できます。
[soak 検証ガイド](docs/soak-validation.md)には実 GUI の churn、8 時間 idle、OS 別メモリ・GPU 計測、
RC 向け sleep/wake 手順もまとめています。通常のテストでは長時間 soak を実行しません。

Windows の DX12 レンダラーは、初期リソース参照領域を wgpu の既定値である100万件から
65,536件に抑えています。フォント・壁紙・透過・履歴を変更せず、起動時の CPU/GPU メモリ確保を
削減します。実際のワーキングセットはグラフィックスドライバーと表示内容にも依存するため、
release ビルドを使い、CPU と GPU のメモリを分けて記録してください。

汎用の `monospace` は文字整形前にインストール済みのフォントへ解決し、使わない等幅フォントを
まとめて読み込んで保持する処理を避けます。代替フォントは設定した順番を優先し、その後に
OS の文字種別のフォントを使います。設定とレジストリは型付きSteel呼出しで取得し、
各フィールド用のソースをコンパイルしません。GC は割り当て量に応じてスクリプトスレッドで実行します。
Steel 0.8.3 の空き領域を拡張してしまうため、リクエストや再読み込みのたびにフル GC を強制しません。
設定・テーマ・登録が変わらないウィジェット更新では、検証済みのネイティブスナップショットを再利用します。
普段の設定を計測するには、soak スクリプトの `gui --scenario idle --config /absolute/path/config.scm`
を使ってください。[メモリ調査記録](docs/performance-baselines/2026-10-13-memory.md)に、
検証結果と macOS での比較手順をまとめています。
[Steel 最適化の計測結果](docs/performance-baselines/2026-10-31-steel-optimization.md)には、
強制 GC の廃止と検証済みスナップショットの再利用を、個人設定で比較した結果を載せています。

ウィンドウからフォーカスが外れると、文字・バー・画像を含む表示全体の色を少し薄くします。
フォーカスが戻ると元の色に戻ります。設定したウィンドウの透過率は維持します。

大きく圧縮しやすい壁紙は、画素を可逆圧縮して CPU メモリに保持します。GPU 転送時は行単位で
小分けに展開して1本の転送用バッファへ直接書き込み、CPU に展開済みの画像全体を確保しません。
GPU 復旧時も同じ画素を再転送し、解像度・色・PNG のアルファ値を保ちます。小さい画像や圧縮効果が
小さい画像は画素をそのまま保持し、通常の再描画では転送済みのテクスチャを再利用します。

[Unicode・IME・描画リグレッションガイド](docs/unicode-rendering-validation.md)に、
自動テストの対象とフォント・IMEの手動検証用fixtureをまとめています。

locked指定の検証コマンド、ネイティブスモークテスト、任意実行の端末性能計測は[開発ガイド](docs/development.md)、
パッケージ作成は[リリースチェックリスト](docs/releasing.md)を参照してください。
[クレート構成](docs/architecture.md)と[スレッド契約](docs/threading.md)で、ネイティブ側の所有権と
専用スクリプトスレッドを説明しています。静的キーバインドはSchemeを呼び出さず、
Schemeコールバックの返すコマンドはメインスレッドで適用します。

## ライセンス

toyotermは[MIT License](LICENSE)で配布します。

SteelはCargo.lockで固定するRust依存です。[サードパーティー通知](THIRD_PARTY_NOTICES.md)を参照してください。Rust依存ライブラリのライセンスはCIで`cargo-deny`を使って検査します。
