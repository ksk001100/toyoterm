# Performance fixture / 性能計測用設定

```sh
cargo run --locked -- --config examples/performance/config.rb
```

Ctrl+Shift+F11 toggles zoom through a native binding; Ctrl+Shift+F12 performs
the same operation through Ruby. This is a measurement fixture, so it does not
load shared navigation or a status bar. Use the logging/measurement procedure
in [performance-baseline.md](../../docs/performance-baseline.md).

Ctrl+Shift+F11はネイティブ、Ctrl+Shift+F12はRuby経由で同じズーム操作を実行します。
計測用なので共通キーやバーは追加しません。
[性能計測手順](../../docs/performance-baseline.md)に沿って使ってください。
