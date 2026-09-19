use super::*;

pub(super) fn collect_registered_plugins(
    runtime: &mut MrubyRuntime,
) -> Result<Vec<PluginMetadata>, ScriptError> {
    let count = runtime
        .eval("Toyoterm.__plugin_count")?
        .parse::<usize>()
        .map_err(|_| ScriptError::new("load plugins", "plugin count is invalid"))?;
    let mut plugins = Vec::with_capacity(count);
    let mut plugin_paths = HashSet::with_capacity(count);
    for index in 0..count {
        let name = runtime.eval(&format!("Toyoterm.__plugin_name({index})"))?;
        let version = runtime.eval(&format!("Toyoterm.__plugin_version({index})"))?;
        let api_requirement = runtime.eval(&format!("Toyoterm.__plugin_requires({index})"))?;
        let path = PathBuf::from(runtime.eval(&format!("Toyoterm.__plugin_path({index})"))?);
        if !plugin_paths.insert(path.clone()) {
            return Err(ScriptError::new(
                "load plugin",
                format!("{} defines more than one plugin", path.display()),
            ));
        }
        parse_semver(&version).map_err(|message| {
            ScriptError::new("load plugin", format!("plugin {name} has {message}"))
        })?;
        if !api_requirement.is_empty()
            && !version_requirement_matches(&api_requirement, PLUGIN_API_VERSION)?
        {
            return Err(ScriptError::new(
                "load plugin",
                format!(
                    "plugin {name} requires toyoterm plugin API `{api_requirement}`, current version is {PLUGIN_API_VERSION}"
                ),
            ));
        }
        plugins.push(PluginMetadata {
            name,
            version,
            api_requirement,
            path,
        });
    }
    Ok(plugins)
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
