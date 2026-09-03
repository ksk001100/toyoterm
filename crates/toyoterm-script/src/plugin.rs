use super::*;

pub(super) fn discover_plugins(directory: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            tracing::warn!(
                target: "toyoterm::script",
                path = %directory.display(),
                %error,
                "cannot scan local plugin directory"
            );
            return Vec::new();
        }
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("rb"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

pub(super) fn load_plugins(
    runtime: &mut MrubyRuntime,
    automatic: &[PathBuf],
    source_dir: Option<&Path>,
) -> Vec<PluginMetadata> {
    let mut queue = automatic
        .iter()
        .cloned()
        .map(|path| (path, None))
        .collect::<VecDeque<_>>();
    queue.extend(drain_plugin_requests(runtime, source_dir));
    let mut loaded_paths = HashSet::new();
    let mut plugins = Vec::new();

    while let Some((path, parent)) = queue.pop_front() {
        let path = resolve_plugin_path(&path, parent.as_deref().or(source_dir));
        let identity = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if !loaded_paths.insert(identity) {
            tracing::debug!(target: "toyoterm::script", path = %path.display(), "skip duplicate plugin path");
            continue;
        }
        match load_plugin(runtime, &path) {
            Ok(metadata) => {
                tracing::info!(
                    target: "toyoterm::script",
                    plugin = %metadata.name,
                    version = %metadata.version,
                    path = %path.display(),
                    "local plugin loaded"
                );
                plugins.push(metadata);
                queue.extend(drain_plugin_requests(runtime, path.parent()));
            }
            Err(error) => {
                tracing::warn!(
                    target: "toyoterm::script",
                    path = %path.display(),
                    %error,
                    "local plugin disabled after load failure"
                );
            }
        }
    }
    plugins
}

pub(super) fn drain_plugin_requests(
    runtime: &mut MrubyRuntime,
    default_parent: Option<&Path>,
) -> Vec<(PathBuf, Option<PathBuf>)> {
    let count = runtime
        .eval("Toyoterm.__plugin_request_count")
        .ok()
        .and_then(|count| count.parse::<usize>().ok())
        .unwrap_or(0);
    let mut requests = Vec::with_capacity(count);
    for index in 0..count {
        let Ok(path) = runtime.eval(&format!("Toyoterm.__plugin_request_path({index})")) else {
            continue;
        };
        let parent = runtime
            .eval(&format!("Toyoterm.__plugin_request_parent({index})"))
            .ok()
            .filter(|parent| !parent.is_empty())
            .and_then(|parent| PathBuf::from(parent).parent().map(Path::to_owned))
            .or_else(|| default_parent.map(Path::to_owned));
        requests.push((PathBuf::from(path), parent));
    }
    let _ = runtime.eval(&format!("Toyoterm.__discard_plugin_requests({count})"));
    requests
}

pub(super) fn resolve_plugin_path(path: &Path, parent: Option<&Path>) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix("~/")
        && let Some(home) = home_directory()
    {
        return home.join(rest);
    }
    if path.is_absolute() {
        path.to_owned()
    } else {
        parent
            .map(|parent| parent.join(path))
            .unwrap_or_else(|| path.to_owned())
    }
}

pub(super) fn load_plugin(
    runtime: &mut MrubyRuntime,
    path: &Path,
) -> Result<PluginMetadata, ScriptError> {
    let source = std::fs::read_to_string(path)
        .map_err(|error| ScriptError::new("load plugin", format!("{}: {error}", path.display())))?;
    let before = runtime
        .eval("Toyoterm.__plugin_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load plugin", "plugin count is invalid"))?;
    runtime.eval("$__toyoterm_plugin_checkpoint = Toyoterm.__plugin_checkpoint")?;
    runtime.eval(&format!(
        "Toyoterm.__begin_plugin({})",
        ruby_string_literal(&path.display().to_string())
    ))?;
    let evaluated = runtime.eval_with_filename(&source, &path.display().to_string());
    let _ = runtime.eval("Toyoterm.__end_plugin");
    if let Err(error) = evaluated {
        let _ = runtime.eval("Toyoterm.__rollback_plugin($__toyoterm_plugin_checkpoint)");
        return Err(ScriptError::new(
            "load plugin",
            format!("{}: {error}", path.display()),
        ));
    }
    let result = (|| {
        let after = runtime
            .eval("Toyoterm.__plugin_count")?
            .parse::<usize>()
            .map_err(|_| ScriptError::new("load plugin", "plugin count is invalid"))?;
        if after != before + 1 {
            return Err(ScriptError::new(
                "load plugin",
                "a plugin file must call Toyoterm::Plugin.define exactly once",
            ));
        }
        let name = runtime.eval(&format!("Toyoterm.__plugin_name({before})"))?;
        let version = runtime.eval(&format!("Toyoterm.__plugin_version({before})"))?;
        let requires = runtime.eval(&format!("Toyoterm.__plugin_requires({before})"))?;
        parse_semver(&version).map_err(|message| {
            ScriptError::new("load plugin", format!("plugin {name} has {message}"))
        })?;
        if !requires.is_empty() && !version_requirement_matches(&requires, PLUGIN_API_VERSION)? {
            return Err(ScriptError::new(
                "load plugin",
                format!(
                    "plugin {name} requires toyoterm plugin API `{requires}`, current version is {PLUGIN_API_VERSION}"
                ),
            ));
        }
        Ok(PluginMetadata {
            name,
            version,
            requires,
            path: path.to_owned(),
        })
    })();
    if result.is_err() {
        let _ = runtime.eval("Toyoterm.__rollback_plugin($__toyoterm_plugin_checkpoint)");
    }
    result
}

pub(super) fn parse_semver(value: &str) -> Result<(u64, u64, u64), String> {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
        })
    {
        return Err(format!("invalid semantic version `{value}`"));
    }
    let parsed = parts
        .iter()
        .map(|part| part.parse::<u64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| format!("invalid semantic version `{value}`"))?;
    Ok((parsed[0], parsed[1], parsed[2]))
}

pub(super) fn version_requirement_matches(
    requirement: &str,
    current: &str,
) -> Result<bool, ScriptError> {
    let current =
        parse_semver(current).map_err(|message| ScriptError::new("load plugin", message))?;
    requirement.split(',').try_fold(true, |matches, clause| {
        let clause = clause.trim();
        let (operator, version) = [">=", "<=", ">", "<", "="]
            .into_iter()
            .find_map(|operator| {
                clause
                    .strip_prefix(operator)
                    .map(|version| (operator, version))
            })
            .unwrap_or(("=", clause));
        let version = parse_semver(version.trim())
            .map_err(|message| ScriptError::new("load plugin", message))?;
        let clause_matches = match operator {
            ">=" => current >= version,
            "<=" => current <= version,
            ">" => current > version,
            "<" => current < version,
            "=" => current == version,
            _ => unreachable!(),
        };
        Ok(matches && clause_matches)
    })
}

pub(super) fn platform_primary_modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "SUPER"
    } else {
        "CTRL"
    }
}

pub(super) fn platform_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "other"
    }
}
