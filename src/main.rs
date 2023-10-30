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

use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::Arc;

use egui::{Ui, Vec2};
use egui_file::FileDialog;
use gsgdt::GraphvizSettings;
use gsgdt::{Edge, Graph, Node, NodeStyle};
use rustc_middle::mir::*;
use rustc_middle::ty::TyCtxt;
use rustc_session::config::{self, Cfg, CheckCfg};

extern crate eframe;
extern crate egui_extras;
use anyhow::Context;
use eframe::egui;
use egui_extras::RetainedImage;
use rustc_driver::Compilation;
use rustc_errors::registry;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_interface::interface::Compiler;
use rustc_interface::Queries;
use std::io::{Read, Write};

struct FnVis {
    pub unique_name: String,
    pub dot: Vec<u8>,
}

struct MyCallbacks<'a> {
    filters: &'a [String],
    graphs: Vec<FnVis>,
}

/// Convert an MIR function into a gsgdt Graph
pub fn mir_fn_to_generic_graph(body: &Body<'_>) -> Graph {
    let def_id = body.source.def_id();
    let def_name = graphviz_safe_def_name(def_id);
    let graph_name = format!("Mir_{}", def_name);

    let nodes: Vec<Node> = body
        .basic_blocks
        .iter_enumerated()
        .map(|(block, _)| bb_to_graph_node(block, body))
        .collect();

    let edges = body
        .basic_blocks
        .iter_enumerated()
        .flat_map(|(source, _)| {
            let terminator = body[source].terminator();
            let labels = terminator.kind.fmt_successor_labels();
            terminator
                .successors()
                .zip(labels)
                .map(move |(target, label)| {
                    let src = node(def_id, source);
                    let trg = node(def_id, target);
                    Edge::new(src, trg, label.to_string())
                })
        })
        .collect();

    Graph::new(graph_name, nodes, edges)
}

fn bb_to_graph_node(block: BasicBlock, body: &Body<'_>) -> Node {
    let def_id = body.source.def_id();
    let data = &body[block];
    let label = node(def_id, block);

    let (title, bgcolor) = if data.is_cleanup {
        (format!("{} (cleanup)", block.index()), "lightblue")
    } else {
        (format!("{}", block.index()), "gray")
    };

    let style = NodeStyle {
        title_bg: Some(bgcolor.to_owned()),
        ..Default::default()
    };

    let mut stmts: Vec<String> = data
        .statements
        .iter() // Filter out the boring ones!
        .filter(|s| s.kind.name() != "StorageLive" && s.kind.name() != "StorageDead")
        .map(|x| format!("{:?}", x))
        .collect();

    // add the terminator to the stmts, gsgdt can print it out separately
    let mut terminator_head = String::new();
    data.terminator()
        .kind
        .fmt_head(&mut terminator_head)
        .unwrap();
    stmts.push(terminator_head);

    Node::new(stmts, label, title, style)
}

// Must match `[0-9A-Za-z_]*`. This does not appear in the rendered graph, so
// it does not have to be user friendly.
pub fn graphviz_safe_def_name(def_id: DefId) -> String {
    format!("{}_{}", def_id.krate.index(), def_id.index.index(),)
}

fn node(def_id: DefId, block: BasicBlock) -> String {
    format!("bb{}__{}", block.index(), graphviz_safe_def_name(def_id))
}

impl<'a> MyCallbacks<'a> {
    pub fn new(filters: &'a [String]) -> Self {
        MyCallbacks {
            filters,
            graphs: Vec::default(),
        }
    }

    fn analyse_with_context(&mut self, tcx: TyCtxt) {
        for def_id in tcx.hir_crate_items(()).definitions() {
            let def_kind = tcx.def_kind(def_id);
            // We only care about function bodies
            if def_kind != DefKind::Fn && def_kind != DefKind::AssocFn {
                continue;
            }
            let fn_name = tcx.item_name(def_id.into());

            if !self.filters.is_empty() && !self.filters.iter().any(|f| f == fn_name.as_str()) {
                continue;
            }
            let graph = mir_fn_to_generic_graph(tcx.optimized_mir(def_id));
            let settings = GraphvizSettings::default();

            let mut dot = Vec::default();
            graph
                .to_dot(&mut dot, &settings, false)
                .expect("Could not render graph!");

            let unique_name = format!("{}-{}", fn_name, graph.name);
            self.graphs.push(FnVis { unique_name, dot });
        }
    }

    fn analyse<'tcx>(&mut self, queries: &'tcx Queries<'tcx>) {
        queries
            .global_ctxt()
            .unwrap()
            .enter(|tcx| self.analyse_with_context(tcx));
    }
}

impl rustc_driver::Callbacks for MyCallbacks<'_> {
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &Compiler,
        queries: &'tcx Queries<'tcx>,
    ) -> Compilation {
        self.analyse(queries);
        Compilation::Stop
    }
}

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

fn rs_to_mir_png(rs_file: &Path, functions: &[String]) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let config = rustc_interface::Config {
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
    };

    let my_cb = rustc_driver::catch_fatal_errors(|| {
        rustc_interface::run_compiler(config, |compiler| {
            let mut my_cb = MyCallbacks::new(functions);
            compiler.enter(|queries| {
                my_cb.analyse(queries);
            });
            my_cb
        })
    })
    .ok()
    .context("Rustc compiler error")?;

    let mut images = Vec::default();

    for f in my_cb.graphs {
        let mut dot_cmd = Command::new("dot")
            .arg("-Tpng")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("Cannot run dot!")?;

        let mut stdout = dot_cmd
            .stdout
            .take()
            .context("Cannot get stdout for dot!")?;

        let mut stdin = dot_cmd.stdin.take().context("Cannot get stdin for dot!")?;
        stdin.write_all(&f.dot).context("Unable to write stdin!")?;
        drop(stdin);

        let mut rendered = Vec::new();

        stdout
            .read_to_end(&mut rendered)
            .context("Unable to read  dot file from stdout!")?;

        images.push((f.unique_name, rendered));
    }

    Ok(images)
}

struct MirWidget {
    label: String,
    render: RetainedImage,
}

impl MirWidget {
    fn new(label: String, img: Vec<u8>) -> Self {
        MirWidget {
            label,
            render: RetainedImage::from_image_bytes("MIR.png", &img).unwrap(),
        }
    }

    fn draw(&self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::top_down(egui::Align::TOP), |ui| {
            egui::ScrollArea::both()
                .id_source(&self.label)
                .show(ui, |ui| {
                    ui.heading(&self.label);
                    self.render
                        .show_max_size(ui, Vec2::splat(ui.available_width()));
                });
        });
    }
}

#[derive(Default)]
struct MirExplorer {
    pub mir_graphs: Vec<MirWidget>,
    opened_file: Option<PathBuf>,
    open_file_dialog: Option<FileDialog>,
    functions: String,
}

impl MirExplorer {
    fn render_button(&mut self, ctx: &egui::Context, ui: &mut Ui) {
        if ui.add(egui::Button::new("Render!")).clicked()
            || ctx.input(|i| i.key_pressed(egui::Key::Enter))
        {
            let fns: Vec<_> = self
                .functions
                .split(' ')
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if let Some(open_file) = &self.opened_file {
                self.mir_graphs = {
                    match rs_to_mir_png(open_file, &fns) {
                        Ok(graphs) => graphs
                            .into_iter()
                            .map(|(name, raw_png)| MirWidget::new(name, raw_png))
                            .collect(),
                        Err(error) => {
                            log::error!("Error generating graphs: {}", error);
                            Vec::default()
                        }
                    }
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
            ui.heading("Filters");
            ui.add(
                egui::TextEdit::singleline(&mut self.functions)
                    .hint_text("Functions (space-separated)"),
            );
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
                            mir.draw(ui);
                        }
                    })
            });
        });
    }
}

fn main() {
    let instance = MirExplorer::new();

    let options = eframe::NativeOptions {
        initial_window_size: None,
        ..Default::default()
    };
    eframe::run_native(
        "Rust MIR visualiser",
        options,
        Box::new(|_cc| Box::new(instance)),
    )
    .expect("CRASHED");
}
