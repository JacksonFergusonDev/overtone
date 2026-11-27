use std::path::PathBuf;

use clap::Subcommand;
use overtone_core::CoreError;
use overtone_core::library::Library;

use crate::output::{Output, table};

#[derive(Debug, Subcommand)]
pub enum RootsCommand {
    /// Index a folder in place. Files are never moved or modified.
    Add { path: PathBuf },
    /// List indexed folders.
    Ls,
    /// Stop indexing a folder. Its tracks stay in the library.
    Rm { id: i64 },
}

pub fn run(library: &Library, command: &RootsCommand, out: &Output) -> Result<(), CoreError> {
    match command {
        RootsCommand::Add { path } => {
            let root = overtone_core::scan::add_root(library, path)?;
            out.emit(&root, || println!("Indexing {}", root.path));
        }
        RootsCommand::Ls => {
            let roots = library.roots()?;
            out.emit(&roots, || {
                if roots.is_empty() {
                    println!("No folders yet. Add one with `overtone-cli roots add <folder>`.");
                    return;
                }
                let mut t = table(&["ID", "Folder"]);
                for root in &roots {
                    t.add_row(vec![root.id.to_string(), root.path.clone()]);
                }
                println!("{t}");
            });
        }
        RootsCommand::Rm { id } => {
            library.remove_root(*id)?;
            out.emit(&serde_json::json!({ "removed": id }), || {
                println!("Stopped indexing folder {id}. Its tracks stay in the library.");
            });
        }
    }
    Ok(())
}
