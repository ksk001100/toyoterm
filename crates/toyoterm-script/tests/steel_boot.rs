use toyoterm_script::{ConfigManager, SteelRuntime};

#[test]
fn steel_boots_and_loads_defaults() {
    let mut runtime = SteelRuntime::new().unwrap();
    assert_eq!(runtime.eval("(+ 20 22)").unwrap(), "42");
    assert_eq!(ConfigManager::new().unwrap().config().font.size, 14.0);
}
