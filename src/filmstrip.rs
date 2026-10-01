use super::*;

impl App {
    pub(super) fn filmstrip(&mut self, ui: &mut egui::Ui) {
        #[cfg(feature = "ui-smoke")]
        {
            self.reel_cells.clear();
            self.reel_menu_items.clear();
        }
        let grid = self.settings.reel_grid;
        let height = ui.available_height().max(70.0);
        let card_width = if grid {
            self.settings.reel_thumbnail_size.clamp(100.0, 300.0)
        } else {
            // Dragging the row's top edge directly changes the preview size.
            ((height - 40.0) * 1.5).clamp(100.0, 540.0)
        };
        let columns = if grid {
            (ui.available_width() / (card_width + 8.0)).floor().max(1.0) as usize
        } else {
            self.visible.len().max(1)
        };
        let layout = crate::reel::Layout {
            columns,
            cell: egui::vec2(
                card_width + 8.0,
                if grid {
                    card_width / 1.5 + 48.0
                } else {
                    height - 10.0
                },
            ),
            count: self.visible.len(),
        };
        let Some(session) = &self.session else {
            return;
        };
        if self.reel_layout != Some((columns, layout.cell)) {
            self.reel_follow = true;
            self.reel_layout = Some((columns, layout.cell));
        }
        let bindings = &self.settings.bindings;
        let button_text = |label: &str, command| {
            let key = COMMANDS
                .iter()
                .find(|s| s.command == command)
                .map(|s| bindings.get(s.id).map_or(s.key, String::as_str))
                .unwrap_or_default();
            format!("{label} [{key}]")
        };
        let mut pins = HashSet::new();
        let mut uploaded = 0;
        let mut upload_bytes = 0;
        let mut chosen = None;
        let mut context_target = None;
        let mut context_command = None;
        let follow = std::mem::take(&mut self.reel_follow);
        let scroll = if grid {
            egui::ScrollArea::vertical()
        } else {
            egui::ScrollArea::horizontal()
        };
        // Map a normal wheel onto the only enabled axis (including Shift+wheel).
        ui.style_mut().always_scroll_the_only_direction = true;
        let result = scroll.id_salt(("reel", grid))
            .auto_shrink([false, false])
            .animated(false)
            .wheel_scroll_multiplier(egui::Vec2::splat(self.settings.reel_scroll_speed.clamp(0.25, 8.0)))
            .show_viewport(ui, |ui, viewport| {
                let total = if grid {
                    egui::vec2(ui.available_width(), self.visible.len().div_ceil(columns) as f32 * layout.cell.y)
                } else { egui::vec2(self.visible.len() as f32 * layout.cell.x, layout.cell.y) };
                ui.set_min_size(total);
                let origin = ui.min_rect().min.to_vec2();
                self.reel_viewport = viewport;
                if follow && let Some(position) = self.visible.iter().position(|i| *i == self.selected) {
                    ui.scroll_to_rect(layout.rect(position).translate(origin), None);
                }
                for position in layout.range(viewport, grid) {
                    let index = self.visible[position];
                    let shot = &session.shots[index];
                    let rect = layout.rect(position).translate(origin);
                    #[cfg(feature = "ui-smoke")]
                    self.reel_cells.push((index, rect));
                    let response = ui.interact(rect, ui.id().with(("shot", index)), egui::Sense::click());
                    if response.clicked() {
                        let modifiers = ui.input(|i| i.modifiers);
                        chosen = Some((index, modifiers.command || modifiers.ctrl, modifiers.shift));
                    }
                    if response.secondary_clicked() { context_target = Some(index); }
                    let selected = self.reel_selection.indices.contains(&index);
                    response.context_menu(|ui| {
                        ui.label(egui::RichText::new(&shot.name).strong());
                        ui.label(format!("{} image(s) selected", if selected { self.reel_selection.indices.len() } else { 1 }));
                        ui.separator();
                        for (label, command) in [("Keep", Command::Keep), ("Reject", Command::Reject), ("Unreviewed", Command::Clear)] {
                            let item = theme::icon_button(ui, &button_text(label, command), theme::command_icon(command).unwrap());
                            #[cfg(feature = "ui-smoke")]
                            self.reel_menu_items.push((command, item.rect));
                            if item.clicked() {
                                context_target = Some(index);
                                context_command = Some(command);
                                ui.close();
                            }
                        }
                        ui.separator();
                        for (label, command) in [("Pin as A", Command::Pin), ("Row / grid", Command::ReelMode), ("Select all", Command::SelectAll), ("Clear selection", Command::DeselectAll)] {
                            let item = theme::icon_button(ui, &button_text(label, command), theme::command_icon(command).unwrap());
                            #[cfg(feature = "ui-smoke")]
                            self.reel_menu_items.push((command, item.rect));
                            if item.clicked() {
                                context_target = Some(index);
                                context_command = Some(command);
                                ui.close();
                            }
                        }
                        if let Some(path) = shot.preview() && ui.button("Copy preview path").clicked() {
                            ui.ctx().copy_text(path.display().to_string());
                            ui.close();
                        }
                    });
                    let decision = shot.decision(shot.review_kind(self.media_filter), self.settings.link_raw);
                    let color = decision_color(decision);
                    ui.painter().rect_filled(rect, 6.0, if selected { theme::SELECTED_BG } else { theme::SURFACE });
                    ui.painter().rect_stroke(rect, 6.0, egui::Stroke::new(if index == self.selected { 3.0 } else { 1.5 }, color), egui::StrokeKind::Inside);
                    if selected {
                        // Blue selection is independent of the green/red/gray decision.
                        ui.painter().rect_stroke(rect.expand(2.0), 7.0, egui::Stroke::new(1.5, theme::SELECTED), egui::StrokeKind::Outside);
                    }
                    if let Some(path) = shot.preview() {
                        let edge = ((card_width * ui.ctx().pixels_per_point()) as u32).div_ceil(128) * 128;
                        let key = Key::fit(path.to_owned(), edge.clamp(128, 1024));
                        pins.insert(key.clone());
                        self.thumbnail_tick += 1;
                        self.thumbnail_ticks.insert(key.clone(), self.thumbnail_tick);
                        self.demands.push((key.clone(), 3));
                        if !self.thumbnails.contains_key(&key) && let Some(picture) = self.cache.get(&key) {
                            let bytes = picture.image.pixels.len() * 4;
                            if uploaded < 4 && upload_bytes + bytes <= 8 * 1024 * 1024 {
                                self.thumbnails.insert(key.clone(), ui.ctx().load_texture(format!("thumb:{}:{edge}", path.display()), picture.image.clone(), egui::TextureOptions::LINEAR));
                                uploaded += 1;
                                upload_bytes += bytes;
                            } else { ui.ctx().request_repaint(); }
                        }
                        if let Some(texture) = self.thumbnails.get(&key) {
                            let area = egui::Rect::from_min_max(rect.min + egui::vec2(6.0, 6.0), rect.max - egui::vec2(6.0, 40.0));
                            let size = texture.size_vec2();
                            let scale = (area.width() / size.x).min(area.height() / size.y);
                            ui.painter().image(texture.id(), egui::Rect::from_center_size(area.center(), size * scale), egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                        }
                    }
                    let name = ui.painter().layout(shot.name.clone(), egui::FontId::proportional(12.0), theme::TEXT, rect.width() - 16.0);
                    ui.painter().with_clip_rect(rect).galley(rect.left_bottom() + egui::vec2(8.0, -34.0), name, theme::TEXT);
                    let detail = format!("{}{}{}",
                        self.burst_groups.get(&index).map(|id| format!("B{id} · ")).unwrap_or_default(),
                        if self.pinned == Some(index) { "A · " } else { "" },
                        if shot.conflict() { "PAIR CONFLICT" } else { decision.label() });
                    ui.painter().with_clip_rect(rect).text(rect.left_bottom() + egui::vec2(8.0, -6.0), egui::Align2::LEFT_BOTTOM, detail, egui::FontId::proportional(11.0), if shot.conflict() { theme::REJECT } else { color });
                    if selected {
                        let mark = rect.right_top() + egui::vec2(-12.0, 12.0);
                        ui.painter().circle_filled(mark, 7.0, theme::SELECTED);
                        ui.painter().line_segment([mark + egui::vec2(-3.0, 0.0), mark + egui::vec2(-1.0, 2.0)], egui::Stroke::new(1.5, theme::BACKGROUND));
                        ui.painter().line_segment([mark + egui::vec2(-1.0, 2.0), mark + egui::vec2(3.0, -2.0)], egui::Stroke::new(1.5, theme::BACKGROUND));
                    }
                    response.on_hover_text(format!("{}\nCtrl-click: toggle selection · Shift-click: range\nRight-click: actions", shot.assets.iter().map(|a| format!("{} · {} · {} bytes", a.name, a.kind.label(), a.size)).collect::<Vec<_>>().join("\n")));
                }
            });
        self.reel_viewport = egui::Rect::from_min_size(
            egui::Pos2::ZERO + result.state.offset,
            result.inner_rect.size(),
        );
        self.reel_clip = result.inner_rect;
        if let Some((index, toggle, range)) = chosen {
            self.reel_selection
                .click(index, &self.visible, toggle, range);
            self.selected = index;
            self.canvas.active_a = false;
        }
        if let Some(index) = context_target {
            if !self.reel_selection.indices.contains(&index) {
                self.reel_selection.single(index);
            }
            self.selected = index;
            self.canvas.active_a = false;
        }
        if let Some(command) = context_command {
            self.command(command);
        }
        let budget = (self.settings.gpu_cache_mib * 1024 * 1024 / 4).max(16 * 1024 * 1024);
        let mut bytes = self
            .thumbnails
            .values()
            .map(|t| t.size()[0] * t.size()[1] * 4)
            .sum::<usize>();
        while self.thumbnails.len() > 256 || bytes > budget {
            let oldest = self
                .thumbnail_ticks
                .iter()
                .filter(|(key, _)| !pins.contains(*key))
                .min_by_key(|(_, tick)| *tick)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                if let Some(texture) = self.thumbnails.remove(&key) {
                    bytes -= texture.size()[0] * texture.size()[1] * 4;
                }
                self.thumbnail_ticks.remove(&key);
            } else {
                break;
            }
        }
    }
}
