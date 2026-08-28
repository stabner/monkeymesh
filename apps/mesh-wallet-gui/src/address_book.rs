//! Local address book for HD wallet indices (next to the vault).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressEntry {
    pub index: u32,
    pub address: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AddressBook {
    pub active_index: u32,
    pub entries: Vec<AddressEntry>,
}

impl AddressBook {
    pub fn path_beside_vault(vault: &Path) -> PathBuf {
        vault.with_extension("addresses.json")
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn ensure_index0(&mut self, address: &str) {
        if self.entries.is_empty() {
            self.entries.push(AddressEntry {
                index: 0,
                address: address.to_string(),
                label: "Primary".into(),
            });
            self.active_index = 0;
        }
    }

    pub fn next_index(&self) -> u32 {
        self.entries.iter().map(|e| e.index).max().unwrap_or(0) + 1
    }

    pub fn active(&self) -> Option<&AddressEntry> {
        self.entries.iter().find(|e| e.index == self.active_index)
    }

    pub fn push(&mut self, index: u32, address: String, label: String) {
        self.entries.push(AddressEntry {
            index,
            address,
            label,
        });
        self.active_index = index;
    }
}

pub fn load_book(vault_path: &Path) -> Result<AddressBook> {
    let path = AddressBook::path_beside_vault(vault_path);
    AddressBook::load(&path).with_context(|| format!("address book {}", path.display()))
}

pub fn save_book(vault_path: &Path, book: &AddressBook) -> Result<()> {
    let path = AddressBook::path_beside_vault(vault_path);
    book.save(&path)
}
