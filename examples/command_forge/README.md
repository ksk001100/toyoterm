# Command Forge / コマンド生成工房

```sh
cargo run --locked -- --config examples/command_forge/config.rb
```

Alt+1/2/3 opens tools, notes or review shells beside the current Pane. Alt+I
shows the actual generated method names, effective keys and command metadata.
Alt+H opens native command help; Alt+O opens the operation palette.
No external program or shell integration is required; reported cwd is inherited
when available. Each invocation adds one shell, and sets its `TOYOTERM_ROLE`.

The interesting part is [forge.rb](forge.rb): one data row becomes a method
through `Module#define_method`, is attached to a private receiver with `extend`,
and becomes a command callback through `Object#method` and `Method#to_proc`.
The same declaration generates the key binding and help metadata. The independent
`forge_launch_requested` hook decorates all generated commands with a badge on
the original Pane. The badge means launch queued, not task completed.
No source-string evaluation or changes to Object/Toyoterm are involved.

Add this row to `entries` in [config.rb](config.rb), reload with Ctrl+Shift+R,
then press Alt+4 and inspect with Alt+I:

```ruby
{ name: :forge_logs, key: '4', direction: :down, role: 'logs',
  description: 'Open a log shell below this Pane' }
```

Use unique command names/keys and valid split directions. Duplicate methods or
keys are not a supported merge mechanism; duplicate command names reject loading.
An invalid generated config leaves the previous config active. Reload replaces
the generated receiver and registrations. [Shared controls](../README.md).

Alt+1／2／3でtools・notes・review用シェルを追加します。Alt+Iで生成されたメソッド・
有効キー・コマンド説明を確認し、Alt+Hでヘルプ、Alt+Oで操作パレットを開けます。
外部プログラムやシェル連携は不要です。cwdが報告されていれば引き継ぎ、子シェルには
`TOYOTERM_ROLE` を設定します。

`forge.rb` は宣言1行から `Module#define_method` でメソッドを作り、`extend` した専用
オブジェクトから `Method` を取り出してコマンドに登録します。同じ宣言からキーとヘルプも
生成します。独立したフックで元のペインに起動要求のバッジを付けます。バッジは起動完了を
示すものではありません。ソース文字列のevalやObject／Toyotermへのモンキーパッチは使いません。

上の宣言を `entries` に加えてCtrl+Shift+Rで再読込し、Alt+4とAlt+Iを試してください。
名前・キーは重複させず、方向はleft／right／up／downを使います。生成した設定が不正なら
以前の設定を保持します。再読込で生成オブジェクトと登録を置き換えます。[共通操作](../README.ja.md)。
