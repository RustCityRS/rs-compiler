use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;

const REPO_URL: &str = "https://github.com/2004Scape/Server.git";
const TEMP_DIR: &str = "2004scape";
const SCRIPTS_PATH: &str = "2004scape/data/src/scripts";
const CONFIGS_PATH: &str = "2004scape/data/src";

#[derive(Debug)]
pub struct ScriptAnalysis {
    pub triggers: HashSet<String>,
    pub commands: HashSet<String>,
    pub types: HashSet<String>,
    pub configs: HashSet<String>,
    pub constants: HashSet<String>,
}

impl ScriptAnalysis {
    pub fn new() -> Self {
        Self {
            triggers: HashSet::new(),
            commands: HashSet::new(),
            types: HashSet::new(),
            configs: HashSet::new(),
            constants: HashSet::new(),
        }
    }

    pub fn analyze_repository(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.setup_repository()?;
        self.analyze_scripts_directory()?;
        self.analyze_configs_directory()?;
        Ok(())
    }

    fn setup_repository(&self) -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = Path::new(TEMP_DIR);
        let git_dir = temp_dir.join(".git");

        if git_dir.exists() {
            println!("Repository exists, checking for updates...");

            // Check if we have any changes
            let status_output = Command::new("git")
                .current_dir(TEMP_DIR)
                .args(&["status", "--porcelain"])
                .output()?;

            if !status_output.stdout.is_empty() {
                println!("Local changes detected, resetting...");
                Command::new("git")
                    .current_dir(TEMP_DIR)
                    .args(&["reset", "--hard", "HEAD"])
                    .output()?;
            }

            // Fetch and check if we're behind
            let fetch_output = Command::new("git")
                .current_dir(TEMP_DIR)
                .args(&["fetch", "origin", "main"])
                .output()?;

            if !fetch_output.status.success() {
                return Err(format!(
                    "Failed to fetch repository: {}",
                    String::from_utf8_lossy(&fetch_output.stderr)
                )
                .into());
            }

            // Check if we need to update
            let rev_list = Command::new("git")
                .current_dir(TEMP_DIR)
                .args(&["rev-list", "HEAD..origin/main", "--count"])
                .output()?;

            let behind_count = String::from_utf8_lossy(&rev_list.stdout)
                .trim()
                .parse::<u32>()
                .unwrap_or(0);

            if behind_count > 0 {
                println!("Updates available, pulling changes...");
                // Pull latest changes
                let pull_output = Command::new("git")
                    .current_dir(TEMP_DIR)
                    .args(&["pull", "origin", "main"])
                    .output()?;

                if !pull_output.status.success() {
                    return Err(format!(
                        "Failed to pull updates: {}",
                        String::from_utf8_lossy(&pull_output.stderr)
                    )
                    .into());
                }
            } else {
                println!("Repository is already up to date!");
            }
        } else {
            println!("Cloning 2004Scape repository...");
            // Create temp directory if it doesn't exist
            if temp_dir.exists() {
                fs::remove_dir_all(temp_dir)?;
            }
            fs::create_dir_all(temp_dir)?;

            let clone_output = Command::new("git")
                .args(&["clone", "--depth", "1", REPO_URL, TEMP_DIR])
                .output()?;

            if !clone_output.status.success() {
                return Err(format!(
                    "Failed to clone repository: {}",
                    String::from_utf8_lossy(&clone_output.stderr)
                )
                .into());
            }
            println!("Repository cloned successfully!");
        }

        Ok(())
    }

    fn walk_directory<F>(
        &mut self,
        dir: &Path,
        callback: &mut F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnMut(&mut Self, &Path),
    {
        if dir.is_dir() {
            let entries = fs::read_dir(dir)?;
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    self.walk_directory(&path, callback)?;
                } else {
                    callback(self, &path);
                }
            }
        }
        Ok(())
    }

    fn analyze_scripts_directory(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("Analyzing scripts directory...");
        let mut callback = |analyzer: &mut Self, path: &Path| {
            if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
                match ext {
                    "rs2" => {
                        println!("  Analyzing script: {}", path.display());
                        if let Ok(contents) = fs::read_to_string(path) {
                            analyzer.analyze_script(&contents);
                        }
                    }
                    "constant" => {
                        println!("  Analyzing constant: {}", path.display());
                        if let Ok(contents) = fs::read_to_string(path) {
                            analyzer.analyze_constant(&contents);
                        }
                    }
                    _ => {}
                }
            }
        };
        self.walk_directory(Path::new(SCRIPTS_PATH), &mut callback)?;
        Ok(())
    }

    fn analyze_configs_directory(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("Analyzing configs...");
        let config_types = [
            "loc",
            "npc",
            "obj",
            "seq",
            "spotanim",
            "varp",
            "param",
            "struct",
            "flo",
            "idk",
            "enum",
            "mesanim",
            "interface",
            "inv",
            "component",
        ];

        for config_type in config_types.iter() {
            let config_path = Path::new(CONFIGS_PATH).join(config_type);
            if config_path.exists() {
                println!("  Analyzing {} configs...", config_type);
                let mut callback = |analyzer: &mut Self, path: &Path| {
                    if path.extension().and_then(|ext| ext.to_str()) == Some(config_type) {
                        println!("    Analyzing file: {}", path.display());
                        if let Ok(contents) = fs::read_to_string(path) {
                            analyzer.analyze_config(&contents, config_type);
                        }
                    }
                };
                self.walk_directory(&config_path, &mut callback)?;
            } else {
                println!("  Config directory not found: {}", config_path.display());
            }
        }
        Ok(())
    }

    fn analyze_script(&mut self, contents: &str) {
        let trigger_pattern = Regex::new(r"\[([\w\d_]+),").unwrap();
        let command_pattern = Regex::new(r"(?m)^(?:[\t ]*)([\w\d_]+)\(").unwrap();
        let type_pattern = Regex::new(r"def_(\w+)").unwrap();
        let gosub_pattern = Regex::new(r"~([\w\d_]+)\(").unwrap();

        for cap in trigger_pattern.captures_iter(contents) {
            if let Some(trigger) = cap.get(1) {
                self.triggers.insert(trigger.as_str().to_string());
            }
        }

        for cap in command_pattern.captures_iter(contents) {
            if let Some(command) = cap.get(1) {
                if !command.as_str().starts_with("def_") {
                    self.commands.insert(command.as_str().to_string());
                }
            }
        }

        for cap in gosub_pattern.captures_iter(contents) {
            if let Some(command) = cap.get(1) {
                self.commands.insert(format!("gosub_{}", command.as_str()));
            }
        }

        for cap in type_pattern.captures_iter(contents) {
            if let Some(type_name) = cap.get(1) {
                self.types.insert(type_name.as_str().to_string());
            }
        }
    }

    fn analyze_constant(&mut self, contents: &str) {
        // Update regex to handle more constant formats
        let constant_patterns = [
            Regex::new(r"^(?m)(?:export\s+)?([A-Z_][A-Z0-9_]*)\s*=").unwrap(), // CONSTANT_NAME =
            Regex::new(r"^(?m)(?:export\s+)?([a-z_][a-z0-9_]*)\s*=").unwrap(), // constant_name =
        ];

        for pattern in constant_patterns.iter() {
            for cap in pattern.captures_iter(contents) {
                if let Some(constant) = cap.get(1) {
                    self.constants.insert(constant.as_str().to_string());
                }
            }
        }
    }

    fn analyze_config(&mut self, contents: &str, config_type: &str) {
        self.configs.insert(config_type.to_string());

        // Update regex patterns for config analysis
        let patterns = [
            Regex::new(r"type\s*=\s*(\w+)").unwrap(),
            Regex::new(r"category\s*=\s*(\w+)").unwrap(),
            Regex::new(r"model\s*=\s*(\w+)").unwrap(),
            Regex::new(r"anim\s*=\s*(\w+)").unwrap(),
            Regex::new(r"param\s*=\s*(\w+)").unwrap(),
        ];

        for pattern in patterns.iter() {
            for cap in pattern.captures_iter(contents) {
                if let Some(type_name) = cap.get(1) {
                    self.types
                        .insert(format!("{}_{}", config_type, type_name.as_str()));
                }
            }
        }
    }

    pub fn print_analysis(&self) {
        println!("\n=== RuneScript Analysis Results ===\n");

        println!("Triggers found ({})", self.triggers.len());
        for trigger in &self.triggers {
            println!("  - {}", trigger);
        }

        println!("\nCommands found ({})", self.commands.len());
        for command in &self.commands {
            println!("  - {}", command);
        }

        println!("\nTypes found ({})", self.types.len());
        for type_name in &self.types {
            println!("  - {}", type_name);
        }

        println!("\nConfig types found ({})", self.configs.len());
        for config in &self.configs {
            println!("  - {}", config);
        }

        println!("\nConstants found ({})", self.constants.len());
        for constant in &self.constants {
            println!("  - {}", constant);
        }
    }
}

impl Drop for ScriptAnalysis {
    fn drop(&mut self) {
        // Clean up temp directory when done
        if Path::new(TEMP_DIR).exists() {
            let _ = fs::remove_dir_all(TEMP_DIR);
        }
    }
}
