use std::fs;
use std::path::Path;

pub fn unit_text(exec: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=Git Forge Cockpit daemon\n\
         Documentation=https://gfc.dev\n\
         After=default.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={exec} daemon run\n\
         Restart=on-failure\n\
         RestartSec=2\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exec = exec.display()
    )
}

pub fn install_user_unit(exec: &Path) -> std::io::Result<std::path::PathBuf> {
    let dir = dirs_config_systemd()?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("gfc.service");
    fs::write(&path, unit_text(exec))?;
    Ok(path)
}

fn dirs_config_systemd() -> std::io::Result<std::path::PathBuf> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME is unset"))?;
    Ok(std::path::PathBuf::from(home).join(".config/systemd/user"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_mentions_user_service() {
        let text = unit_text(Path::new("/usr/bin/gfc"));
        assert!(text.contains("ExecStart=/usr/bin/gfc daemon run"));
        assert!(text.contains("WantedBy=default.target"));
    }
}
