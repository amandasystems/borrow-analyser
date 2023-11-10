#![feature(rustc_private)]
#![deny(rustc::internal)]
extern crate rustc_driver;
extern crate rustc_error_codes;
extern crate rustc_errors;
extern crate rustc_hash;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_log;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{self};
use std::sync::Arc;

use egui::epaint::{CubicBezierShape, PathShape};
use egui::{
    vec2, Color32, Label, Pos2, Rect, RichText, Rounding, Sense, Shape, Stroke, TextStyle, Ui,
    Vec2, Widget,
};
use egui_file::FileDialog;

use rustc_middle::mir::*;

use rustc_session::config::{self, Cfg, CheckCfg};

extern crate eframe;
extern crate egui_extras;
use anyhow::Context;
use eframe::egui;
use rustc_errors::registry;
use rustc_hir::def::DefKind;

fn get_sysroot() -> Option<std::path::PathBuf> {
    let out = process::Command::new("rustc")
        .arg("--print=sysroot")
        .current_dir(".")
        .output()
        .ok()?;

    String::from_utf8(out.stdout)
        .map(|s| PathBuf::from(s.trim()))
        .ok()
}

fn config_from_file(rs_file: &Path) -> rustc_interface::Config {
    rustc_interface::Config {
        opts: config::Options {
            edition: rustc_span::edition::Edition::Edition2024,
            maybe_sysroot: get_sysroot(),
            ..config::Options::default()
        },
        input: config::Input::File(rs_file.to_path_buf()),
        using_internal_features: Arc::new(false.into()),
        crate_cfg: Cfg::default(),
        crate_check_cfg: CheckCfg::default(),
        output_dir: None,
        output_file: None,
        file_loader: None,
        locale_resources: rustc_driver::DEFAULT_LOCALE_RESOURCES,
        lint_caps: rustc_hash::FxHashMap::default(),
        parse_sess_created: None,
        register_lints: None,
        override_queries: None,
        registry: registry::Registry::new(rustc_error_codes::DIAGNOSTICS),
        make_codegen_backend: None,
        expanded_args: Vec::default(),
        ice_file: None,
        hash_untracked_state: None,
    }
}

fn get_rs_functions(rs_file: &Path) -> anyhow::Result<Vec<String>> {
    rustc_driver::catch_fatal_errors(|| {
        rustc_interface::run_compiler(config_from_file(rs_file), |compiler| {
            let mut fn_names: Vec<String> = Vec::default();
            compiler.enter(|queries| {
                queries.global_ctxt().unwrap().enter(|tcx| {
                    for def_id in tcx.hir_crate_items(()).definitions() {
                        let def_kind = tcx.def_kind(def_id);

                        if def_kind != DefKind::Fn && def_kind != DefKind::AssocFn {
                            continue;
                        }
                        let fn_name = tcx.item_name(def_id.into());

                        fn_names.push(fn_name.to_string());
                    }
                });
            });
            fn_names
        })
    })
    .ok()
    .context("Rustc compiler error")
}

fn rs_to_mir(rs_file: &Path, functions: &[String]) -> anyhow::Result<Vec<MirBody>> {
    rustc_driver::catch_fatal_errors(|| {
        rustc_interface::run_compiler(config_from_file(rs_file), |compiler| {
            let mut mir_bodies: Vec<MirBody> = Vec::default();
            compiler.enter(|queries| {
                queries.global_ctxt().unwrap().enter(|tcx| {
                    for def_id in tcx.hir_crate_items(()).definitions() {
                        let def_kind = tcx.def_kind(def_id);

                        if def_kind != DefKind::Fn && def_kind != DefKind::AssocFn {
                            continue;
                        }
                        let fn_name = tcx.item_name(def_id.into());

                        if !functions.contains(&fn_name.as_str().to_owned()) {
                            // FIXME: move this logic!
                            continue;
                        }

                        mir_bodies.push(MirBody::new(
                            fn_name.as_str().to_string(),
                            tcx.optimized_mir(def_id),
                        ));
                    }
                });
            });
            mir_bodies
        })
    })
    .ok()
    .context("Rustc compiler error")
}

struct MirEdge {
    from: BasicBlock,
    to: BasicBlock,
}

impl MirEdge {
    fn draw(&self, rects: &HashMap<BasicBlock, Rect>, ui: &Ui) {
        let from = *rects.get(&self.from).unwrap();
        let to = rects.get(&self.to).unwrap();

        let src_pos = from.center_bottom();
        let dst_pos = to.center_top() - vec2(10.0, 10.0);
        let stroke = egui::Stroke {
            width: 2.0,
            color: Color32::from_rgb(96, 70, 59),
        };

        let control_scale = ((dst_pos.x - src_pos.x) / 2.0).max(30.0);
        let src_control = src_pos + Vec2::X * control_scale;
        let dst_control = dst_pos - Vec2::X * control_scale;

        let bezier = CubicBezierShape::from_points_stroke(
            [src_pos, src_control, dst_control, dst_pos],
            false,
            Color32::TRANSPARENT,
            stroke,
        );

        ui.painter().add(bezier);
        ui.painter().circle_stroke(src_pos, 2.0, stroke);

        ui.painter().add(end_triangle(
            dst_pos,
            to.center_top() - vec2(2.0, 2.0),
            0.5,
            stroke.color,
            stroke,
        ));
    }
}

struct MirNode {
    id: BasicBlock,
    name: String,
    statements: Vec<String>,
}

impl Widget for &MirNode {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let margin = egui::vec2(15.0, 5.0);
        let gunmetal = Color32::from_rgb(46, 49, 56);

        let background_shape = ui.painter().add(Shape::Noop);
        let outer_rect_bounds = ui.available_rect_before_wrap();
        let mut inner_rect = outer_rect_bounds.shrink2(margin);
        inner_rect.max.x = inner_rect.max.x.max(inner_rect.min.x);
        inner_rect.max.y = inner_rect.max.y.max(inner_rect.min.y);

        let mut child_ui = ui.child_ui(inner_rect, *ui.layout());

        let mut node_rect = outer_rect_bounds;

        child_ui.vertical(|ui| {
            ui.add(Label::new(
                RichText::new(&self.name).text_style(TextStyle::Button),
            ));
            ui.add_space(margin.y);
            ui.add(Label::new(RichText::new(self.statements.join("\n")).code()));
            node_rect = ui.min_rect();
        });

        let box_rect = node_rect.expand2(margin);

        let box_shape = Shape::Rect(egui::epaint::RectShape {
            rect: box_rect,
            rounding: Rounding::same(5.0),
            fill: Color32::from_rgb(204, 201, 231),
            stroke: Stroke {
                width: 1.0,
                color: gunmetal,
            },
        });

        ui.painter().set(background_shape, box_shape);
        ui.allocate_rect(box_rect, Sense::click())
    }
}

struct NodeRow {
    nodes: Vec<MirNode>,
}

impl NodeRow {
    fn draw(&self, ui: &mut Ui) -> Vec<(BasicBlock, Rect)> {
        let mut rects = Vec::new();
        ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
            for node in self.nodes.iter() {
                let response = ui.add(node);
                rects.push((node.id, response.rect));
                ui.add_space(5.0) // FIXME standardise
            }
        });
        rects
    }
}

struct MirBody {
    label: String,
    rows: Vec<NodeRow>,
    edges: Vec<MirEdge>,
}

impl MirBody {
    fn new(fn_name: String, body: &Body<'_>) -> MirBody {
        let mut rows: Vec<NodeRow> = Vec::new();
        let mut edges: Vec<MirEdge> = Vec::new();

        let root = body.basic_blocks.iter_enumerated().next().unwrap();
        let mut backlog = vec![root];
        let mut seen: HashSet<_> = HashSet::new();

        while !backlog.is_empty() {
            let this_row = backlog;
            backlog = Vec::new();

            for (idx, node) in this_row.iter() {
                let terminator = node.terminator();

                for successor in terminator.successors() {
                    edges.push(MirEdge {
                        from: *idx,
                        to: successor,
                    });

                    if seen.insert(successor) {
                        backlog.push((successor, &body[successor]));
                    }
                }
            }
            rows.push(NodeRow {
                nodes: this_row
                    .into_iter()
                    .map(|(idx, node)| {
                        let mut statements: Vec<_> = node
                            .statements
                            .iter() // Filter out the boring ones!
                            .filter(|s| {
                                s.kind.name() != "StorageLive" && s.kind.name() != "StorageDead"
                            })
                            .map(|x| format!("{:?}", x))
                            .collect();
                        let mut terminator_head = String::new();
                        node.terminator()
                            .kind
                            .fmt_head(&mut terminator_head)
                            .unwrap();
                        statements.push(terminator_head);

                        let name = format!("bb {}", idx.index());

                        MirNode {
                            id: idx,
                            name,
                            statements,
                        }
                    })
                    .collect(),
            });
        }

        MirBody {
            label: fn_name,
            rows,
            edges,
        }
    }
}

impl Widget for &MirBody {
    // FIXME move the selected/not selected booleans here!
    fn ui(self, ui: &mut Ui) -> egui::Response {
        ui.with_layout(egui::Layout::top_down(egui::Align::TOP), |ui| {
            egui::ScrollArea::both()
                .id_source(&self.label)
                .show(ui, |ui| {
                    ui.with_layout(egui::Layout::top_down(egui::Align::TOP), |ui| {
                        ui.heading(&self.label);
                        let mut node_to_widget = HashMap::default();
                        for row in self.rows.iter() {
                            node_to_widget.extend(row.draw(ui));
                            ui.add_space(20.0);
                        }

                        for edge in self.edges.iter() {
                            edge.draw(&node_to_widget, ui);
                        }
                    });
                });
        })
        .response
    }
}

#[derive(Default)]
struct MirExplorer {
    pub mir_graphs: Vec<MirBody>,
    opened_file: Option<PathBuf>,
    open_file_dialog: Option<FileDialog>,
    functions: Vec<String>,
    fn_selected: Vec<bool>,
}

impl MirExplorer {
    fn render_button(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        if ui.add(egui::Button::new("Render!")).clicked()
            || ctx.input(|i| i.key_pressed(egui::Key::Enter))
        {
            if let Some(open_file) = &self.opened_file {
                self.mir_graphs = {
                    let filtered_fns: Vec<String> = self
                        .functions
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| self.fn_selected[*i])
                        .map(|(_, fnn)| fnn.clone())
                        .collect();

                    rs_to_mir(open_file, &filtered_fns).ok().unwrap() // FIXME
                }
            }
        }
    }

    fn new() -> Self {
        MirExplorer::default()
    }
}

impl eframe::App for MirExplorer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::SidePanel::left("my_left_panel").show(ctx, |ui| {
            ui.heading("Functions");
            for (i, fn_name) in self.functions.iter().enumerate() {
                ui.checkbox(&mut self.fn_selected[i], fn_name);
            }
        });
        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            egui::Grid::new("main-controls")
                .num_columns(3)
                .show(ui, |ui| {
                    let file_name = self
                        .opened_file
                        .as_ref()
                        .and_then(|f| f.to_str())
                        .unwrap_or("No file!");
                    ui.label(file_name);
                    if ui.button("Open").clicked() {
                        let mut dialog = FileDialog::open_file(self.opened_file.clone());
                        dialog.open();
                        self.open_file_dialog = Some(dialog);
                    };

                    if let Some(dialog) = &mut self.open_file_dialog {
                        if dialog.show(ctx).selected() {
                            if let Some(file) = dialog.path() {
                                self.functions = get_rs_functions(file).unwrap(); // FIXME
                                self.fn_selected = Vec::with_capacity(self.functions.len());
                                for _ in self.functions.iter() {
                                    self.fn_selected.push(false)
                                }
                                self.opened_file = Some(file.to_path_buf());
                            }
                        }
                    };

                    self.render_button(ctx, ui);
                });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                egui::ScrollArea::horizontal()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for mir in self.mir_graphs.iter() {
                            ui.add(mir);
                        }
                    })
            });
        });
    }
}

fn end_triangle(
    bottom_c: Pos2,
    tip: Pos2,
    bottom_scale: f32,
    fill: Color32,
    stroke: Stroke,
) -> PathShape {
    let bottom_tip_v = tip - bottom_c;
    let bottom = bottom_c + bottom_tip_v.rot90() * bottom_scale;
    let top = bottom_c + bottom_tip_v.rot90().rot90().rot90() * bottom_scale;
    egui::epaint::PathShape::convex_polygon(vec![top, tip, bottom, top], fill, stroke)
}

fn main() {
    let instance = MirExplorer::new();
    eframe::run_native(
        "Rust MIR visualiser",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Box::new(instance)),
    )
    .expect("CRASHED");
}
