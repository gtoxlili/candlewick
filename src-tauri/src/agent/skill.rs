//! The skill that tells coding agents about the API: written into the skill
//! folders of the agents found on this machine, and removed again when AI
//! access is turned off.

use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

const NAME: &str = "candlewick";
/// Marks a skill folder as ours, so removing it never touches the user's own.
const MARKER: &str = ".candlewick";
const TEMPLATE: &str = include_str!("SKILL.md");

/// A skill folder agents read, and who reads it.
struct Folder {
    skills: PathBuf,
    agents: &'static [&'static str],
}

/// Where the agents keep their settings when an environment variable says
/// otherwise (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`).
type Env = dyn Fn(&str) -> Option<OsString>;

/// The skill folders of the agents installed under `home`: Claude Code's
/// (which OpenCode reads too), Codex's, and OpenCode's own only when there is
/// no Claude Code, so OpenCode never sees the skill twice.
fn folders(home: &Path, env: &Env) -> Vec<Folder> {
    let dir = |var: &str, default: &str| env(var).map_or_else(|| home.join(default), PathBuf::from);
    let claude = dir("CLAUDE_CONFIG_DIR", ".claude");
    let codex = dir("CODEX_HOME", ".codex");
    let opencode = home.join(".config").join("opencode");
    let mut found = Vec::new();
    match (claude.is_dir(), opencode.is_dir()) {
        (true, true) => found
            .push(Folder { skills: claude.join("skills"), agents: &["Claude Code", "OpenCode"] }),
        (true, false) => {
            found.push(Folder { skills: claude.join("skills"), agents: &["Claude Code"] })
        }
        (false, true) => {
            found.push(Folder { skills: opencode.join("skills"), agents: &["OpenCode"] })
        }
        (false, false) => {}
    }
    if codex.is_dir() {
        found.push(Folder { skills: codex.join("skills"), agents: &["Codex"] });
    }
    found
}

/// Writes the skill for the API at `url`, its token in `token`, into each
/// agent's folder. Returns the agents that got it.
pub fn install(home: &Path, url: &str, token: &Path) -> io::Result<Vec<&'static str>> {
    install_in(&folders(home, &|var| env::var_os(var)), url, token)
}

fn install_in(folders: &[Folder], url: &str, token: &Path) -> io::Result<Vec<&'static str>> {
    let text = render(url, token);
    let mut agents = Vec::new();
    for folder in folders {
        let dir = folder.skills.join(NAME);
        // Someone else's skill by this name stays as it is.
        if dir.exists() && !dir.join(MARKER).exists() {
            log::warn!("{} is not ours; leaving it", dir.display());
            continue;
        }
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(MARKER), b"")?;
        fs::write(dir.join("SKILL.md"), &text)?;
        agents.extend(folder.agents);
    }
    Ok(agents)
}

/// Removes the skill from every folder it may have been written to.
pub fn uninstall(home: &Path) {
    uninstall_from(home, &|var| env::var_os(var));
}

fn uninstall_from(home: &Path, env: &Env) {
    let mut folders: Vec<PathBuf> = folders(home, env).into_iter().map(|f| f.skills).collect();
    // Where it may sit from before the agents found changed.
    folders.extend([
        home.join(".claude/skills"),
        home.join(".codex/skills"),
        home.join(".config/opencode/skills"),
    ]);
    for skills in folders {
        let dir = skills.join(NAME);
        if dir.join(MARKER).exists()
            && let Err(e) = fs::remove_dir_all(&dir)
        {
            log::warn!("cannot remove {}: {e}", dir.display());
        }
    }
}

/// The skill text for this machine: the API's address, where the token is,
/// and a request in this system's shell.
fn render(url: &str, token: &Path) -> String {
    let token = token.display().to_string();
    let (shell, example) = if cfg!(windows) {
        let path = token.replace('\'', "''");
        (
            "powershell",
            format!(
                "curl.exe -s -H \"Authorization: Bearer $(Get-Content -Raw '{path}')\" '{url}/v1/portfolio'"
            ),
        )
    } else {
        let path = token.replace('\'', r"'\''");
        ("sh", format!("curl -s -H \"Authorization: Bearer $(cat '{path}')\" '{url}/v1/portfolio'"))
    };
    TEMPLATE
        .replace("{{url}}", url)
        .replace("{{token}}", &token)
        .replace("{{shell}}", shell)
        .replace("{{example}}", &example)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skill_names_this_machines_api() {
        let text = render(
            "http://127.0.0.1:52733",
            Path::new("/Users/a/Library/Application Support/x/agent-token"),
        );
        assert!(!text.contains("{{"));
        assert!(text.starts_with("---\nname: candlewick\n"));
        #[cfg(unix)]
        assert!(text.contains(
            "curl -s -H \"Authorization: Bearer $(cat '/Users/a/Library/Application Support/x/agent-token')\" 'http://127.0.0.1:52733/v1/portfolio'"
        ));
    }

    #[test]
    fn installs_where_agents_live_and_removes_only_its_own() {
        let home = env::temp_dir().join(format!("candlewick-skill-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join(".claude/skills")).unwrap();
        fs::create_dir_all(home.join(".config/opencode")).unwrap();
        // Someone else's skill by the same name, in Codex's folder.
        fs::create_dir_all(home.join(".codex/skills/candlewick")).unwrap();

        let none = |_: &str| None;
        let agents =
            install_in(&folders(&home, &none), "http://127.0.0.1:1", Path::new("/t")).unwrap();
        assert_eq!(agents, ["Claude Code", "OpenCode"]);
        assert!(home.join(".claude/skills/candlewick/SKILL.md").exists());
        assert!(!home.join(".config/opencode/skills").exists());
        assert!(!home.join(".codex/skills/candlewick/SKILL.md").exists());

        uninstall_from(&home, &none);
        assert!(!home.join(".claude/skills/candlewick").exists());
        assert!(home.join(".codex/skills/candlewick").exists());
        fs::remove_dir_all(&home).unwrap();
    }
}
