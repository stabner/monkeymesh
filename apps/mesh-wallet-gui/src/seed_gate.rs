//! Unlock / create / restore BIP39 vault gate.

use eframe::egui::{self, Color32, RichText, TextureHandle, Vec2};

use crate::chrome::paint_backdrop;
use crate::theme::{ghost_btn, label_upper, panel, pointer, primary_btn, CYAN, DANGER, INK, MUTED};
use crate::wallet_store::{
    create_vault, legacy_exists, load_legacy_key, migrate_legacy_to_vault, restore_vault,
    unlock_vault, vault_exists, LoadedWallet,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GateTab {
    Unlock,
    Create,
    Restore,
    Legacy,
}

pub struct SeedGate {
    pub tab: GateTab,
    pub password: String,
    pub password2: String,
    pub phrase: String,
    pub error: String,
    pub has_vault: bool,
    pub has_legacy: bool,
}

impl SeedGate {
    pub fn new() -> Self {
        let has_vault = vault_exists();
        let has_legacy = legacy_exists();
        let tab = if has_vault {
            GateTab::Unlock
        } else if has_legacy {
            GateTab::Legacy
        } else {
            GateTab::Create
        };
        Self {
            tab,
            password: String::new(),
            password2: String::new(),
            phrase: String::new(),
            error: String::new(),
            has_vault,
            has_legacy,
        }
    }

    /// Returns unlocked wallet when successful.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        hero: Option<&TextureHandle>,
        mascot: Option<&TextureHandle>,
    ) -> Option<LoadedWallet> {
        let mut unlocked = None;
        ctx.request_repaint_after(std::time::Duration::from_millis(400));
        let time = ctx.input(|i| i.time);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(9, 14, 20)))
            .show(ctx, |ui| {
                paint_backdrop(ui, hero, time);
                ui.vertical_centered(|ui| {
                    ui.add_space(48.0);
                    if let Some(tex) = mascot {
                        ui.add(egui::Image::new(tex).fit_to_exact_size(Vec2::splat(72.0)));
                    }
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("MonkeyMesh")
                            .color(CYAN)
                            .size(22.0)
                            .family(crate::theme::ui_family())
                            .strong(),
                    );
                    ui.label(
                        RichText::new("Vault")
                            .color(MUTED)
                            .size(13.0)
                            .family(crate::theme::ui_family()),
                    );
                    ui.label(
                        RichText::new("BIP39 seed · Argon2id · ChaCha20-Poly1305")
                            .color(MUTED)
                            .size(12.0),
                    );
                    ui.add_space(18.0);

                    panel().show(ui, |ui| {
                        ui.set_width(ui.available_width().min(400.0));
                        ui.horizontal(|ui| {
                            tab_btn(ui, &mut self.tab, GateTab::Unlock, "Unlock", self.has_vault);
                            tab_btn(ui, &mut self.tab, GateTab::Create, "Create", true);
                            tab_btn(ui, &mut self.tab, GateTab::Restore, "Restore", true);
                            if self.has_legacy {
                                tab_btn(ui, &mut self.tab, GateTab::Legacy, "Legacy", true);
                            }
                        });
                        ui.add_space(12.0);

                        match self.tab {
                            GateTab::Unlock => {
                                ui.label(
                                    RichText::new("Enter your vault password to unlock the seed.")
                                        .color(INK)
                                        .size(13.0),
                                );
                                ui.add_space(8.0);
                                label_upper(ui, "Password");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.password)
                                        .password(true)
                                        .desired_width(380.0),
                                );
                                ui.add_space(12.0);
                                if primary_btn(ui, "Unlock vault", true).clicked() {
                                    match unlock_vault(&self.password) {
                                        Ok(w) => unlocked = Some(w),
                                        Err(e) => self.error = e.to_string(),
                                    }
                                }
                            }
                            GateTab::Create => {
                                ui.label(
                                    RichText::new(
                                        "Creates a new 24-word BIP39 seed, encrypted on disk. Write the words down offline.",
                                    )
                                    .color(INK)
                                    .size(13.0),
                                );
                                ui.add_space(8.0);
                                label_upper(ui, "New password (min 15 — passphrase, no complexity rules)");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.password)
                                        .password(true)
                                        .desired_width(380.0),
                                );
                                label_upper(ui, "Confirm password");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.password2)
                                        .password(true)
                                        .desired_width(380.0),
                                );
                                ui.add_space(12.0);
                                if primary_btn(ui, "Generate secure wallet", true).clicked() {
                                    if self.password != self.password2 {
                                        self.error = "passwords do not match".into();
                                    } else {
                                        match create_vault(&self.password) {
                                            Ok(w) => unlocked = Some(w),
                                            Err(e) => self.error = e.to_string(),
                                        }
                                    }
                                }
                            }
                            GateTab::Restore => {
                                ui.label(
                                    RichText::new(
                                        "Restore from an existing 12–24 word seed phrase.",
                                    )
                                    .color(INK)
                                    .size(13.0),
                                );
                                ui.add_space(8.0);
                                label_upper(ui, "Seed phrase");
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.phrase)
                                        .desired_width(380.0)
                                        .desired_rows(3),
                                );
                                label_upper(ui, "New vault password");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.password)
                                        .password(true)
                                        .desired_width(380.0),
                                );
                                ui.add_space(12.0);
                                if primary_btn(ui, "Restore wallet", true).clicked() {
                                    match restore_vault(&self.phrase, &self.password) {
                                        Ok(w) => unlocked = Some(w),
                                        Err(e) => self.error = e.to_string(),
                                    }
                                }
                            }
                            GateTab::Legacy => {
                                ui.label(
                                    RichText::new(
                                        "Legacy plaintext key detected. Unlock it, then migrate into an encrypted vault.",
                                    )
                                    .color(DANGER)
                                    .size(13.0),
                                );
                                ui.add_space(8.0);
                                if ghost_btn(ui, "Open legacy key (insecure)").clicked() {
                                    match load_legacy_key() {
                                        Ok(w) => unlocked = Some(w),
                                        Err(e) => self.error = e.to_string(),
                                    }
                                }
                                ui.add_space(8.0);
                                label_upper(ui, "Password to encrypt legacy key");
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.password)
                                        .password(true)
                                        .desired_width(380.0),
                                );
                                if primary_btn(ui, "Migrate to encrypted vault", true).clicked() {
                                    match migrate_legacy_to_vault(&self.password) {
                                        Ok(w) => {
                                            self.has_vault = true;
                                            unlocked = Some(w);
                                        }
                                        Err(e) => self.error = e.to_string(),
                                    }
                                }
                            }
                        }

                        if !self.error.is_empty() {
                            ui.add_space(10.0);
                            ui.label(RichText::new(&self.error).color(DANGER).size(12.0));
                        }
                    });

                    ui.add_space(16.0);
                    ui.label(
                        RichText::new("Never share your seed. MonkeyMesh staff will never ask for it.")
                            .color(MUTED)
                            .size(11.0),
                    );
                });
            });

        unlocked
    }
}

fn tab_btn(ui: &mut egui::Ui, current: &mut GateTab, tab: GateTab, label: &str, enabled: bool) {
    if !enabled {
        return;
    }
    let selected = *current == tab;
    let fill = if selected {
        Color32::from_rgba_unmultiplied(0, 245, 255, 40)
    } else {
        Color32::TRANSPARENT
    };
    let resp = egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(
            1.0,
            if selected {
                CYAN
            } else {
                Color32::from_rgb(40, 60, 70)
            },
        ))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.label(
                RichText::new(label)
                    .color(if selected { CYAN } else { MUTED })
                    .size(12.0),
            );
        })
        .response
        .interact(egui::Sense::click());
    let resp = pointer(resp);
    if resp.clicked() {
        *current = tab;
    }
}
