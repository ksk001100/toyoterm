# Pane workbench / ペイン作業台

```sh
cargo run --locked -- --config examples/pane_workbench/config.rb
```

Alt+W adds right and lower default-shell Panes relative to the original Pane,
both in its reported cwd. Their environments set `TOYOTERM_ROLE` to `tools`
and `scratch`. Repeating the command adds two more shells. Alt+N opens a tab
in the same directory with role `project`. No editor or watcher is required;
edit the launch specifications to use direct argv for your own tools.

Alt+P lists up to 4096 Panes across workspaces. Filter by cwd, title or ID and
press Enter to activate the full workspace/window/tab/Pane path. Closed targets
are ignored. Try splitting, opening a tab, then jumping to the original Pane.
Ctrl+Shift+Z zooms the selected Pane. Enable [shell integration](../../docs/shell-integration.md)
for accurate cwd; without it the normal inherited directory is used.
See [shared controls](../README.md).

Alt+Wで元のペインの右と下に既定シェルを開きます。報告されたcwdを引き継ぎ、
環境変数 `TOYOTERM_ROLE` をそれぞれ `tools`／`scratch` にします。繰り返すとさらに
2つ追加します。Alt+Nは同じcwdで役割 `project` のタブを開きます。エディタや監視ツールの
インストールは不要です。好みのツールは起動指定を直接argvに変更して使えます。

Alt+Pはワークスペースをまたいで最大4096ペインを一覧表示し、cwd・タイトル・IDで
絞り込んでEnterで移動できます。待機中に閉じた対象は無視します。分割・タブ作成後、
元のペインへ戻る操作を試してください。Ctrl+Shift+Zでズームできます。
正確なcwdには[シェル連携](../../docs/shell-integration.md)が必要です。連携なしでは
通常の継承先ディレクトリを使います。[共通操作](../README.ja.md)も参照してください。
