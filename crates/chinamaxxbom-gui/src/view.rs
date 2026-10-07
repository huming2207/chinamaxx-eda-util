use anyhow::{Context as _, Result};
use chinamaxxbom_core::{
    board::Board,
    export,
    library::{Library, Target},
    project::{Edit, Project},
    supplier::{self, Part},
};
use gpui::*;
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::{h_flex, v_flex, ActiveTheme, Disableable, Selectable};
use std::path::{Path, PathBuf};

enum JobResult {
    Loaded(Project, Board, PathBuf),
    Search(supplier::Search),
    Message(String),
}
pub struct BomView {
    path: Entity<InputState>,
    out: Entity<InputState>,
    query: Entity<InputState>,
    catalog: Entity<InputState>,
    cache_days: Entity<InputState>,
    lcsc: Entity<InputState>,
    dx: Entity<InputState>,
    dy: Entity<InputState>,
    rotation: Entity<InputState>,
    bundle: Entity<InputState>,
    import_id: Entity<InputState>,
    lib_out: Entity<InputState>,
    project: Option<Project>,
    board: Option<Board>,
    project_path: Option<PathBuf>,
    selected: Option<String>,
    results: Vec<Part>,
    tab: u8,
    busy: bool,
    status: String,
    error: Option<String>,
    basic: bool,
    stock: bool,
    page: u32,
    target: Target,
    dnp: bool,
    allow_missing: bool,
    gerbers: bool,
}
fn text(state: &Entity<InputState>, cx: &App) -> String {
    state.read(cx).value().to_string()
}
impl BomView {
    pub fn new(initial: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cache_settings = chinamaxxbom_core::cache::Cache::for_user();
        let cache_days = cache_settings
            .as_ref()
            .map(|c| c.ttl_days())
            .unwrap_or(7)
            .to_string();
        let mut input =
            |value: &str| cx.new(|cx| InputState::new(window, cx).default_value(value.to_owned()));
        let mut v = Self {
            path: input(initial.as_deref().unwrap_or("")),
            out: input("output/assembly"),
            query: input(""),
            catalog: input(""),
            cache_days: input(&cache_days),
            lcsc: input(""),
            dx: input("0"),
            dy: input("0"),
            rotation: input("0"),
            bundle: input(""),
            import_id: input(""),
            lib_out: input("output/library"),
            project: None,
            board: None,
            project_path: None,
            selected: None,
            results: vec![],
            tab: 0,
            busy: false,
            status: "Open an Eagle or KiCad project, schematic, or board to start.".into(),
            error: cache_settings.err().map(|e| format!("{e:#}")),
            basic: false,
            stock: true,
            page: 1,
            target: Target::Both,
            dnp: false,
            allow_missing: false,
            gerbers: false,
        };
        if initial.is_some() {
            v.open(cx);
        }
        v
    }
    fn job(
        &mut self,
        label: &str,
        cx: &mut Context<Self>,
        work: impl FnOnce() -> Result<JobResult> + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = label.into();
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { work() }).await;
            let _ = this.update(cx, |v, cx| {
                v.busy = false;
                match result {
                    Ok(JobResult::Loaded(p, b, path)) => {
                        v.status = format!(
                            "{} components · {} · assignments saved in {}",
                            b.parts.len(),
                            p.input.as_ref().unwrap_or(&p.board).display(),
                            path.display()
                        );
                        v.project = Some(p);
                        v.board = Some(b);
                        v.project_path = Some(path);
                    }
                    Ok(JobResult::Search(r)) => {
                        v.status = format!(
                            "{} matches · {} shown · page {} · {}",
                            r.total,
                            r.results.len(),
                            v.page,
                            match &r.cache {
                                Some(info) if info.hit => format!(
                                    "cached response ({}-day TTL)",
                                    (info.expires_at - info.fetched_at) / 86400
                                ),
                                Some(_) => "JLC API response".into(),
                                None => "offline catalogue".into(),
                            }
                        );
                        v.results = r.results;
                    }
                    Ok(JobResult::Message(s)) => v.status = s,
                    Err(e) => {
                        v.error = Some(format!("{e:#}"));
                        v.status = "Operation failed. See the error below.".into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn open(&mut self, cx: &mut Context<Self>) {
        let path = PathBuf::from(text(&self.path, cx));
        self.selected = None;
        self.project = None;
        self.board = None;
        self.project_path = None;
        self.job("Opening board…", cx, move || {
            let (project, project_path) = Project::open(&path)?;
            let board = project.snapshot()?;
            Ok(JobResult::Loaded(project, board, project_path))
        });
    }
    fn browse(&mut self, which: u8, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: which == 0,
            multiple: false,
            prompt: Some("Choose file".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = match receiver.await {
                Ok(Ok(Some(paths))) => paths,
                _ => return,
            };
            let path = match paths.first() {
                Some(path) => path,
                None => return,
            };
            let path = path.display().to_string();
            let _ = this.update_in(cx, |v, window, cx| {
                let state = match which {
                    0 => v.path.clone(),
                    1 => v.catalog.clone(),
                    _ => v.bundle.clone(),
                };
                state.update(cx, |s, cx| s.set_value(path, window, cx));
                if which == 0 {
                    v.open(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn select(&mut self, reference: String, window: &mut Window, cx: &mut Context<Self>) {
        let p = match self
            .board
            .as_ref()
            .and_then(|b| b.parts.iter().find(|p| p.reference == reference))
        {
            Some(p) => p,
            None => return,
        };
        let e = self
            .project
            .as_ref()
            .and_then(|p| p.edits.get(&reference))
            .cloned()
            .unwrap_or_default();
        self.lcsc
            .update(cx, |s, cx| s.set_value(p.lcsc.clone(), window, cx));
        for (state, value) in [
            (&self.dx, e.dx),
            (&self.dy, e.dy),
            (&self.rotation, e.rotation),
        ] {
            state.update(cx, |s, cx| s.set_value(value.to_string(), window, cx));
        }
        self.dnp = p.dnp;
        self.selected = Some(reference);
        cx.notify();
    }
    fn pick(&mut self, id: String, cx: &mut Context<Self>) {
        let (mut project, path, reference) =
            match (&self.project, &self.project_path, &self.selected) {
                (Some(p), Some(path), Some(r)) => (p.clone(), path.clone(), r.clone()),
                _ => return,
            };
        self.job("Saving selected part…", cx, move || {
            project.pick(&path, &reference, &id)?;
            let bom = project.snapshot()?;
            Ok(JobResult::Loaded(project, bom, path))
        });
    }
    fn apply(&mut self, cx: &mut Context<Self>) {
        let mut p = match self.project.clone() {
            Some(p) => p,
            None => return,
        };
        let reference = match self.selected.clone() {
            Some(r) => r,
            None => return,
        };
        let path = match self.project_path.clone() {
            Some(p) => p,
            None => return,
        };
        let id = text(&self.lcsc, cx);
        let dx = text(&self.dx, cx);
        let dy = text(&self.dy, cx);
        let r = text(&self.rotation, cx);
        let dnp = self.dnp;
        self.job("Saving assignment…", cx, move || {
            let e = Edit {
                lcsc: if id.trim().is_empty() {
                    None
                } else {
                    Some(chinamaxxbom_core::lcsc_id(&id)?)
                },
                dnp: Some(dnp),
                dx: chinamaxxbom_core::finite(&dx)?,
                dy: chinamaxxbom_core::finite(&dy)?,
                rotation: chinamaxxbom_core::finite(&r)?,
            };
            p.edit(&reference, e)?;
            p.save(&path)?;
            let b = p.snapshot()?;
            Ok(JobResult::Loaded(p, b, path))
        });
    }
    fn search(&mut self, cx: &mut Context<Self>) {
        let query = text(&self.query, cx);
        let catalog = text(&self.catalog, cx);
        let basic = self.basic;
        let stock = self.stock;
        let page = self.page;
        self.job("Searching parts…", cx, move || {
            Ok(JobResult::Search(if catalog.trim().is_empty() {
                supplier::search(&query, page, basic, stock)?
            } else {
                supplier::search_catalog(Path::new(&catalog), &query, basic, stock)?
            }))
        });
    }
    fn import(&mut self, cx: &mut Context<Self>) {
        let bundle = text(&self.bundle, cx);
        let id = text(&self.import_id, cx);
        let out = text(&self.lib_out, cx);
        let target = self.target;
        self.job("Importing library…", cx, move || {
            let b = if bundle.trim().is_empty() {
                supplier::fetch(&id)?
            } else {
                serde_json::from_slice(&std::fs::read(&bundle).context("Read cached bundle")?)?
            };
            let report = Library::parse(&b)?.write(Path::new(&out), target)?;
            Ok(JobResult::Message(format!(
                "Imported {}: {} pads, {} pins, {} units into {}. {}",
                report.lcsc,
                report.pads,
                report.pins,
                report.units,
                out,
                report.warnings.join(" ")
            )))
        });
    }
    fn board_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_1();
        if let Some(board) = &self.board {
            for p in &board.parts {
                let reference = p.reference.clone();
                let selected = self.selected.as_ref() == Some(&reference);
                let label = format!(
                    "{:<8} {:<24} {:<12} {:<7} {}",
                    p.reference,
                    p.value,
                    if p.lcsc.is_empty() {
                        "Unassigned"
                    } else {
                        &p.lcsc
                    },
                    if p.side == chinamaxxbom_core::board::Side::Top {
                        "Top"
                    } else {
                        "Bottom"
                    },
                    if p.dnp { "DNP" } else { "" }
                );
                rows = rows.child(
                    Button::new(SharedString::from(format!("part-{}", p.reference)))
                        .label(label)
                        .w_full()
                        .selected(selected)
                        .on_click(cx.listener(move |v, _, window, cx| {
                            v.select(reference.clone(), window, cx)
                        }))
                        .disabled(self.busy),
                );
            }
        } else {
            rows = rows.child(
                div()
                    .p_6()
                    .text_color(cx.theme().muted_foreground)
                    .child("Open a project, schematic, board, or saved JSON above."),
            );
        }
        let details=v_flex().w(px(315.)).flex_shrink_0().gap_3().p_4().bg(cx.theme().muted).rounded_lg()
            .child(div().text_lg().child(self.selected.clone().unwrap_or("Select a component".into())))
            .child("LCSC part number").child(Input::new(&self.lcsc).disabled(self.busy||self.selected.is_none()))
            .child(h_flex().gap_2().child(Button::new("dnp").label(if self.dnp{"☑ Do not populate"}else{"☐ Do not populate"}).selected(self.dnp).disabled(self.busy).on_click(cx.listener(|v,_,_,cx|{v.dnp=!v.dnp;cx.notify();}))))
            .child("Assembly offset X / Y (mm)")
            .child(h_flex().gap_2().child(Input::new(&self.dx).disabled(self.busy)).child(Input::new(&self.dy).disabled(self.busy)))
            .child("Rotation correction (degrees)").child(Input::new(&self.rotation).disabled(self.busy))
            .child(Button::new("apply").label("Apply & save assignment").primary().disabled(self.busy||self.selected.is_none()).on_click(cx.listener(|v,_,_,cx|v.apply(cx))))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Offsets use assembly world axes: X right, Y up. Changes are saved in the project sidecar."));
        v_flex().gap_3().flex_1().min_h_0()
            .child(h_flex().gap_3().flex_1().min_h_0().items_start().child(div().id("parts-scroll").flex_1().h_full().overflow_y_scroll().child(rows)).child(details))
            .child(h_flex().gap_2().child(div().w(px(145.)).child("New output folder")).child(Input::new(&self.out).disabled(self.busy)))
            .child(h_flex().gap_2()
                .child(Button::new("missing").label("Allow missing parts").selected(self.allow_missing).disabled(self.busy).on_click(cx.listener(|v,_,_,cx|{v.allow_missing=!v.allow_missing;cx.notify();})))
                .child(Button::new("gerbers").label("Include KiCad Gerbers").selected(self.gerbers).disabled(self.busy || !self.project.as_ref().is_some_and(|p| p.has_placement())).on_click(cx.listener(|v,_,_,cx|{v.gerbers=!v.gerbers;cx.notify();})))
                .child(Button::new("export").label(if self.project.as_ref().is_some_and(|p| p.has_placement()) { "Export BOM + CPL" } else { "Export BOM" }).primary().disabled(self.busy||self.project.is_none()).on_click(cx.listener(|v,_,_,cx|{
                    let p = match v.project.clone() { Some(p) => p, None => return };let out=text(&v.out,cx);let missing=v.allow_missing;let gerbers=v.gerbers && p.has_placement();
                    v.job("Exporting assembly…",cx,move||{let r=export::export(&p,Path::new(&out),missing,gerbers)?;Ok(JobResult::Message(format!("Exported {} placements to {}. {}",r.included,out,r.warnings.join(" "))))});
                }))))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Footprint/element origins are used for placement. Verify component centres and rotations in JLCPCB's preview before ordering."))
            .into_any_element()
    }
    fn search_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().gap_2();
        for (i, p) in self.results.iter().enumerate() {
            let id = p.lcsc.clone();
            let import_id = p.lcsc.clone();
            let datasheet = p.datasheet.clone();
            rows = rows.child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .child(div().child(format!(
                        "{}   {}   {}   {}",
                        p.lcsc, p.model, p.brand, p.package
                    )))
                    .child(div().text_sm().child(format!(
                            "{} · stock {} · unit price {} · {}",
                            if p.basic { "Basic" } else { "Extended" },
                            p.stock,
                            p.price
                                .map(|v| format!("{v:.4}"))
                                .unwrap_or("unknown".into()),
                            p.description
                        )))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(SharedString::from(format!("assign-{i}")))
                                    .label("Use for selected component")
                                    .disabled(self.busy || self.selected.is_none())
                                    .on_click(cx.listener(move |v, _, w, cx| {
                                        v.lcsc.update(cx, |s, cx| s.set_value(id.clone(), w, cx));
                                        v.tab = 0;
                                        v.pick(id.clone(), cx);
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("import-{i}")))
                                    .label("Import library…")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(move |v, _, w, cx| {
                                        v.import_id.update(cx, |s, cx| {
                                            s.set_value(import_id.clone(), w, cx)
                                        });
                                        v.bundle.update(cx, |s, cx| s.set_value("", w, cx));
                                        v.tab = 2;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(SharedString::from(format!("datasheet-{i}")))
                                    .label("Datasheet")
                                    .disabled(!datasheet.starts_with("https://"))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        if datasheet.starts_with("https://") {
                                            cx.open_url(&datasheet);
                                        }
                                    })),
                            ),
                    ),
            );
        }
        v_flex()
            .gap_3()
            .flex_1()
            .min_h_0()
            .child(format!("Selecting for: {}", self.selected.as_deref().unwrap_or("select a component in Assembly")))
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.query).disabled(self.busy))
                    .child(
                        Button::new("search")
                            .label("Search parts")
                            .primary()
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.page = 1;
                                v.search(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("basic")
                            .label("Basic only")
                            .selected(self.basic)
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.basic = !v.basic;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("stock")
                            .label("In stock only")
                            .selected(self.stock)
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.stock = !v.stock;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("prev")
                            .label("Previous page")
                            .disabled(self.busy || self.page <= 1)
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.page -= 1;
                                v.search(cx);
                            })),
                    )
                    .child(
                        Button::new("next")
                            .label("Next page")
                            .disabled(self.busy || !text(&self.catalog, cx).is_empty())
                            .on_click(cx.listener(|v, _, _, cx| {
                                v.page += 1;
                                v.search(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().w(px(190.)).child("Offline catalogue (optional)"))
                    .child(Input::new(&self.catalog).disabled(self.busy))
                    .child(
                        Button::new("catalog")
                            .label("Browse…")
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, w, cx| v.browse(1, w, cx))),
                    ),
            )
            .child(h_flex().gap_2()
                .child("Cache TTL (7–60 days)")
                .child(div().w(px(90.)).child(Input::new(&self.cache_days).disabled(self.busy)))
                .child(Button::new("save-cache-days").label("Save").disabled(self.busy)
                    .on_click(cx.listener(|v, _, _, cx| {
                        let value = text(&v.cache_days, cx);
                        v.job("Saving cache setting…", cx, move || {
                            let days = value.trim().parse::<u64>().context("Enter a whole number of days")?;
                            chinamaxxbom_core::cache::Cache::save_ttl_days(days)?;
                            Ok(JobResult::Message(format!("Cache TTL saved: {days} days (GUI and CLI)")))
                        });
                    }))))
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("Search runs on request. Cached stock and prices may be as old as the configured TTL."))
            .child(
                div()
                    .id("search-results")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(rows),
            )
            .into_any_element()
    }
    fn library_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex().gap_4().max_w(px(800.))
            .child(div().text_xl().child("Import a supplier library"))
            .child("LCSC ID (online)").child(Input::new(&self.import_id).disabled(self.busy))
            .child("Or a saved EasyEDA bundle JSON (offline)")
            .child(h_flex().gap_2().child(Input::new(&self.bundle).disabled(self.busy)).child(Button::new("bundle").label("Browse…").disabled(self.busy).on_click(cx.listener(|v,_,w,cx|v.browse(2,w,cx)))))
            .child("Create library in a new folder").child(Input::new(&self.lib_out).disabled(self.busy))
            .child(h_flex().gap_2().children([(Target::Both,"Both"),(Target::Kicad,"KiCad 10+"),(Target::Eagle,"Eagle 9")].into_iter().enumerate().map(|(i,(t,label))|Button::new(SharedString::from(format!("target-{i}"))).label(label).selected(self.target==t).disabled(self.busy).on_click(cx.listener(move|v,_,_,cx|{v.target=t;cx.notify();})))))
            .child(Button::new("import-library").label("Import symbol + footprint").primary().disabled(self.busy).on_click(cx.listener(|v,_,_,cx|v.import(cx))))
            .child(div().text_color(cx.theme().muted_foreground).child("KiCad: add the generated .kicad_sym and .pretty libraries in Preferences → Manage Libraries. Eagle: open the generated .lbr in Library Manager. Import reports list omitted decorative text and unsupported 3D assets. Unsupported pads and pin mappings stop the import."))
            .into_any_element()
    }
}
impl Render for BomView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = match self.tab {
            1 => self.search_panel(cx),
            2 => self.library_panel(cx),
            _ => self.board_panel(cx),
        };
        let content = v_flex()
            .flex_1()
            .min_h_0()
            .p_5()
            .gap_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        v_flex()
                            .child(div().text_2xl().child("ChinamaxxBOM"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Eagle + KiCad 10 · Parts, libraries & assembly"),
                            ),
                    )
                    .child(div().child(if self.busy { "Working…" } else { "Ready" })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.path).disabled(self.busy))
                    .child(
                        Button::new("browse-board")
                            .label("Browse…")
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, w, cx| v.browse(0, w, cx))),
                    )
                    .child(
                        Button::new("open-board")
                            .label("Open project / schematic")
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, _, cx| v.open(cx))),
                    ),
            )
            .child(
                h_flex().gap_2().children(
                    ["Assembly", "Parts catalogue", "Library import"]
                        .into_iter()
                        .enumerate()
                        .map(|(i, label)| {
                            Button::new(SharedString::from(format!("tab-{i}")))
                                .label(label)
                                .selected(self.tab == i as u8)
                                .on_click(cx.listener(move |v, _, _, cx| {
                                    v.tab = i as u8;
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .child(panel)
            .child(
                div()
                    .id("status")
                    .max_h(px(140.))
                    .overflow_y_scroll()
                    .text_sm()
                    .child(self.error.clone().unwrap_or_else(|| self.status.clone()))
                    .text_color(if self.error.is_some() {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    }),
            );
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(gpui_component::TitleBar::new().child("ChinamaxxBOM"))
            .child(content)
    }
}
