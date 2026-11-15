//! Whole-process filesystem confinement; CLI permission prompts are additional
//! owner policy, never the boundary protecting another room's plaintext.
use super::StartError;
use crate::owned::SharedProject;
use hagency_platform::Launch;
use std::path::Path;

/// With a space's shared project folder, the process works in that folder
/// and may read and write it besides its own private scope; it still reaches
/// no other room's scope and no other space's folder.
pub(super) fn confine(
    launch: &mut Launch,
    scope: &Path,
    project: Option<&SharedProject>,
) -> Result<(), StartError> {
    confine_paths(
        launch,
        scope,
        project,
        &["HOME", "CLAUDE_CONFIG_DIR", "TMPDIR", "CLAUDE_CODE_TMPDIR"],
    )
}

/// Octos runs with a private, ephemeral copy of exactly one selected
/// profile. The caller still has to bind pool custody and a live lease.
#[cfg(target_os = "macos")]
pub(in crate::owned) fn confine_octos(
    launch: &mut Launch,
    scope: &Path,
    project: Option<&SharedProject>,
) -> Result<(), StartError> {
    if launch.directory.canonicalize().ok().as_deref() != Some(&launch.directory) {
        return Err(StartError::Settings);
    }
    // The short spelling avoids the Unix socket path limit; its resolved
    // target remains inside the confined room. It grants no extra filesystem
    // access, including to the private parent of the alias.
    let runtime = launch.environment.get(std::ffi::OsStr::new("XDG_RUNTIME_DIR"))
        .map(Path::new).ok_or(StartError::Settings)?;
    if !runtime.starts_with("/private/tmp")
        || runtime.canonicalize().ok().as_deref() != Some(scope.join("tmp").as_path()) {
        return Err(StartError::Settings);
    }
    confine_paths(launch, scope, project, &["HOME", "OCTOS_HOME", "TMPDIR", "XDG_CONFIG_HOME"])
}

fn confine_paths(
    launch: &mut Launch,
    scope: &Path,
    project: Option<&SharedProject>,
    names: &[&str],
) -> Result<(), StartError> {
    let working = match project {
        Some(project) => {
            project.check()
                && project.apart_from(scope)
                && launch.directory == project.directory()
        }
        None => launch.directory.starts_with(scope),
    };
    if !scope.is_absolute() || scope.canonicalize().ok().as_deref() != Some(scope) || !working {
        return Err(StartError::Settings);
    }
    for name in names {
        let value = launch
            .environment
            .get(std::ffi::OsStr::new(*name))
            .ok_or(StartError::Settings)?;
        let path = Path::new(value);
        if !path.starts_with(scope) || path.canonicalize().ok().as_deref() != Some(path) {
            return Err(StartError::Settings);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let executable = launch
            .executable
            .canonicalize()
            .map_err(|_| StartError::Settings)?;
        let policy = seatbelt(scope, &executable, project.map(SharedProject::directory))?;
        let mut arguments = vec!["-p".into(), policy.into(), executable.into_os_string()];
        arguments.append(&mut launch.arguments);
        launch.executable = "/usr/bin/sandbox-exec".into();
        launch.arguments = arguments;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(StartError::Unsupported)
    }
}

#[cfg(target_os = "macos")]
fn seatbelt(scope: &Path, executable: &Path, project: Option<&Path>) -> Result<String, StartError> {
    let quote = |path: &Path| {
        serde_json::to_string(path.to_str().ok_or(StartError::Settings)?)
            .map_err(|_| StartError::Settings)
    };
    // The space's shared project folder, when there is one, is the only
    // directory besides the private scope that the process may use.
    let scope = match project {
        Some(project) => format!("(subpath {}) (subpath {})", quote(scope)?, quote(project)?),
        None => format!("(subpath {})", quote(scope)?),
    };
    let executable = quote(executable)?;
    // Native CLI startup reads the OS directory database and timezone. None
    // of these rules admits user homes, other room scopes, owner policy,
    // Keychain services, process environments or shared temporary files.
    Ok(format!(
        r#"(version 1)
(allow default)
(deny file-read*)
(allow file-read-metadata)
(allow file-read* (literal "/") (subpath "/System/Library")
 (subpath "/System/iOSSupport") (subpath "/System/Volumes/Preboot")
 (subpath "/usr") (subpath "/bin") (subpath "/sbin")
 (subpath "/Library/Apple") (subpath "/dev") (subpath "/private/etc")
 (subpath "/private/var/db/timezone") (subpath "/private/var/db/DarwinDirectory")
 (subpath "/private/var/db/eligibilityd") (subpath "/private/var/run")
 {scope} (literal {executable}))
(deny file-read* (subpath "/usr/local"))
(allow file-read* (literal {executable}))
(deny file-write*)
(allow file-write* {scope} (literal "/dev/null") (subpath "/dev/fd"))
(deny process-info* (target others))
(deny mach-lookup (global-name-prefix "com.apple.security"))
(deny mach-priv-task-port)
(deny ipc-posix-shm*)
(deny ipc-posix-sem*)
"#
    ))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn shared_claude_sandbox_denies_other_scopes_and_owner_files() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let scope = root.join("room");
        std::fs::create_dir(&scope).unwrap();
        let own = scope.join("own");
        let other = root.join("other-room");
        std::fs::write(&own, "SCOPED").unwrap();
        std::fs::write(&other, "PRIVATE").unwrap();
        let command = format!(
            "cat '{}' && ! cat '{}' && ! printf overwrite > '{}'",
            own.display(),
            other.display(),
            other.display()
        );
        let result = std::process::Command::new("/usr/bin/sandbox-exec")
            .args([
                "-p",
                &seatbelt(&scope, Path::new("/bin/sh"), None).unwrap(),
                "/bin/sh",
                "-c",
                &command,
            ])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .current_dir(&scope)
            .output()
            .unwrap();
        assert!(result.status.success(), "{:?}", result.status);
        assert_eq!(result.stdout, b"SCOPED");
        assert_eq!(std::fs::read_to_string(other).unwrap(), "PRIVATE");
    }

    #[test]
    fn shared_project_folder_is_open_but_other_spaces_and_scopes_stay_closed() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let scope = root.join("state/room");
        let projects = root.join("Projects");
        let project = projects.join("app");
        let other_space = projects.join("other");
        let other_room = root.join("state/other-room");
        for path in [&scope, &project, &other_space, &other_room] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::write(project.join("main.rs"), "SHARED").unwrap();
        std::fs::write(other_space.join("secret"), "SPACE").unwrap();
        std::fs::write(other_room.join("secret"), "ROOM").unwrap();
        let command = format!(
            "cat main.rs && printf new > added && ! cat '{}' && ! cat '{}' \
             && ! printf x > '{}' && printf own > '{}'",
            other_space.join("secret").display(),
            other_room.join("secret").display(),
            other_space.join("written").display(),
            scope.join("own").display(),
        );
        let result = std::process::Command::new("/usr/bin/sandbox-exec")
            .args([
                "-p",
                &seatbelt(&scope, Path::new("/bin/sh"), Some(&project)).unwrap(),
                "/bin/sh",
                "-c",
                &command,
            ])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .current_dir(&project)
            .output()
            .unwrap();
        assert!(result.status.success(), "{:?} {}", result.status, String::from_utf8_lossy(&result.stderr));
        assert_eq!(result.stdout, b"SHARED");
        assert_eq!(std::fs::read_to_string(project.join("added")).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(scope.join("own")).unwrap(), "own");
        assert!(!other_space.join("written").exists());
    }

    #[test]
    fn a_launch_outside_the_scope_needs_the_exact_project_folder() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let scope = root.join("state/room");
        let projects = root.join("Projects");
        let folder = projects.join("app");
        for path in [&scope, &folder, &projects.join("other")] {
            std::fs::create_dir_all(path).unwrap();
        }
        for name in ["home", "config", "tmp"] {
            std::fs::create_dir(scope.join(name)).unwrap();
        }
        let project = SharedProject::new(projects.clone(), folder.clone()).unwrap();
        let launch = |directory: &Path| {
            let mut environment = std::collections::BTreeMap::new();
            for (name, sub) in [("HOME", "home"), ("CLAUDE_CONFIG_DIR", "config"), ("TMPDIR", "tmp"), ("CLAUDE_CODE_TMPDIR", "tmp")] {
                environment.insert(name.into(), scope.join(sub).into_os_string());
            }
            Launch {
                executable: "/bin/sh".into(),
                arguments: Vec::new(),
                directory: directory.to_path_buf(),
                environment,
                require_crash_containment: false,
            }
        };
        assert!(confine(&mut launch(&folder), &scope, None).is_err());
        assert!(confine(&mut launch(&projects.join("other")), &scope, Some(&project)).is_err());
        assert!(confine(&mut launch(&scope.join("home")), &scope, Some(&project)).is_err());
        let mut confined = launch(&folder);
        confine(&mut confined, &scope, Some(&project)).unwrap();
        assert_eq!(confined.executable, Path::new("/usr/bin/sandbox-exec"));
        // A project root that contains the private scope is refused.
        let around = SharedProject::new(root.clone(), root.join("state")).unwrap();
        assert!(confine(&mut launch(&root.join("state")), &scope, Some(&around)).is_err());
    }
}
