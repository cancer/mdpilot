use directories::ProjectDirs;
use std::path::PathBuf;

pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Option<Self> {
        let pd = ProjectDirs::from("dev", "mdpilot", "mdpilot")?;
        Some(Self {
            config_dir: pd.config_dir().to_path_buf(),
            data_dir: pd.data_dir().to_path_buf(),
            cache_dir: pd.cache_dir().to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_returns_paths_in_environments_with_home() {
        let paths = AppPaths::resolve().expect("HOME is expected on dev/CI machines");
        for (label, p) in [
            ("config", &paths.config_dir),
            ("data", &paths.data_dir),
            ("cache", &paths.cache_dir),
        ] {
            let rendered = p.to_string_lossy();
            assert!(
                rendered.to_ascii_lowercase().contains("mdpilot"),
                "{label} dir {rendered:?} should reference the app identifier",
            );
        }
    }
}
