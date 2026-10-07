use crate::tables::{BomTable, CatalogTable};
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
use gpui_component::tab::{Tab, TabBar};
use gpui_component::table::{Table, TableEvent, TableState};
use gpui_component::{checkbox::Checkbox, scroll::ScrollableElement, Sizable};
use gpui_component::{h_flex, v_flex, ActiveTheme, Disableable, Selectable};
use gpui_component::{IconName, Theme, ThemeMode};
use std::path::{Path, PathBuf};

enum JobResult {
    Loaded(Project, Board, PathBuf),
    Search(supplier::Search),
    Message(String),
}
pub struct BomView {
    bom_table: Entity<TableState<BomTable>>,
    catalog_table: Entity<TableState<CatalogTable>>,
    catalog_selection: Option<usize>,
    _subscriptions: Vec<Subscription>,
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
        let bom_table = cx.new(|cx| {
            TableState::new(BomTable::new(), window, cx)
                .col_movable(false)
                .col_selectable(false)
        });
        let catalog_table = cx.new(|cx| {
            TableState::new(CatalogTable::new(), window, cx)
                .col_movable(false)
                .col_selectable(false)
        });
        let subscriptions = vec![
            cx.subscribe_in(&bom_table, window, |v, table, event, window, cx| {
                if let TableEvent::SelectRow(index) = event {
                    let reference = table
                        .read(cx)
                        .delegate()
                        .rows
                        .get(*index)
                        .map(|p| p.reference.clone());
                    if let Some(reference) = reference {
                        v.select(reference, window, cx);
                    }
                }
            }),
            cx.subscribe_in(&catalog_table, window, |v, _, event, _, cx| {
                if let TableEvent::SelectRow(index) = event {
                    v.catalog_selection = Some(*index);
                    cx.notify();
                }
            }),
        ];
        let cache_settings = chinamaxxbom_core::cache::Cache::for_user();
        let cache_days = cache_settings
            .as_ref()
            .map(|c| c.ttl_days())
            .unwrap_or(7)
            .to_string();
        let mut input =
            |value: &str| cx.new(|cx| InputState::new(window, cx).default_value(value.to_owned()));
        let mut v = Self {
            bom_table,
            catalog_table,
            catalog_selection: None,
            _subscriptions: subscriptions,
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
                        v.bom_table.update(cx, |t, cx| {
                            t.delegate_mut().set_rows(b.parts.clone());
                            t.refresh(cx);
                            cx.notify();
                        });
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
                        v.catalog_selection = None;
                        v.catalog_table.update(cx, |t, cx| {
                            t.clear_selection(cx);
                            t.delegate_mut().rows = r.results.clone();
                            t.refresh(cx);
                            cx.notify();
                        });
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
        self.bom_table.update(cx, |t, cx| {
            t.delegate_mut().rows.clear();
            t.clear_selection(cx);
            t.refresh(cx);
            cx.notify();
        });
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
                    Some(String::new())
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
        let selected = self.selected.as_ref().and_then(|r| {
            self.board
                .as_ref()?
                .parts
                .iter()
                .find(|p| &p.reference == r)
        });
        let disabled = self.busy || selected.is_none();
        let (count, assigned) = self
            .board
            .as_ref()
            .map(|b| {
                let parts: Vec<_> = b
                    .parts
                    .iter()
                    .filter(|p| !p.dnp && !p.exclude_bom)
                    .collect();
                (
                    parts.len(),
                    parts.iter().filter(|p| !p.lcsc.is_empty()).count(),
                )
            })
            .unwrap_or((0, 0));
        let details = v_flex().gap_4().p_4()
            .child(v_flex().gap_1()
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("COMPONENT"))
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(self.selected.clone().unwrap_or("No selection".into())))
                .child(div().text_sm().child(selected.map(|p| p.value.clone()).unwrap_or("Select a table row to inspect it.".into())))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(selected.map(|p| p.footprint.clone()).unwrap_or_default())))
            .child(Button::new("find-part").label("Find JLCPCB part").primary().w_full().disabled(disabled)
                .on_click(cx.listener(|v,_,w,cx| {
                    if let Some(part) = v.selected.as_ref().and_then(|r| v.board.as_ref()?.parts.iter().find(|p| &p.reference == r)) {
                        let query = part.value.clone();
                        v.query.update(cx,|s,cx|s.set_value(query,w,cx));
                    }
                    v.tab=1;cx.notify();
                })))
            .child(v_flex().gap_2().child(div().text_sm().child("LCSC part number")).child(Input::new(&self.lcsc).disabled(disabled)))
            .child(Checkbox::new("dnp").label("Do not populate").checked(self.dnp).disabled(disabled)
                .on_click(cx.listener(|v,checked,_,cx| {v.dnp=*checked;cx.notify();})))
            .child(v_flex().gap_3().pt_3().border_t_1().border_color(cx.theme().border)
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("PLACEMENT CORRECTIONS"))
                .child(h_flex().gap_2()
                    .child(v_flex().flex_1().gap_1().child(div().text_sm().child("X offset · mm")).child(Input::new(&self.dx).disabled(disabled)))
                    .child(v_flex().flex_1().gap_1().child(div().text_sm().child("Y offset · mm")).child(Input::new(&self.dy).disabled(disabled))))
                .child(v_flex().gap_1().child(div().text_sm().child("Rotation · degrees")).child(Input::new(&self.rotation).disabled(disabled))))
            .child(Button::new("apply").label("Save changes").w_full().disabled(disabled).on_click(cx.listener(|v,_,_,cx|v.apply(cx))))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("JLCPCB picks save automatically. Manual changes use Save changes. Check placement in JLCPCB’s preview before ordering."));
        let footer = v_flex()
            .flex_shrink_0()
            .gap_3()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_3()
                    .child(div().text_sm().w(px(100.)).child("Output folder"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.out).disabled(self.busy)),
                    )
                    .child(
                        Button::new("export")
                            .label(
                                if self.project.as_ref().is_some_and(|p| p.has_placement()) {
                                    "Export BOM + CPL"
                                } else {
                                    "Export BOM"
                                },
                            )
                            .primary()
                            .disabled(self.busy || self.project.is_none())
                            .on_click(cx.listener(|v, _, _, cx| {
                                let p = match v.project.clone() {
                                    Some(p) => p,
                                    None => return,
                                };
                                let out = text(&v.out, cx);
                                let missing = v.allow_missing;
                                let gerbers = v.gerbers && p.has_placement();
                                v.job("Exporting assembly…", cx, move || {
                                    let r = export::export(&p, Path::new(&out), missing, gerbers)?;
                                    Ok(JobResult::Message(format!(
                                        "Exported {} placements to {}. {}",
                                        r.included,
                                        out,
                                        r.warnings.join(" ")
                                    )))
                                });
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_5()
                    .child(
                        Checkbox::new("missing")
                            .label("Allow unassigned parts")
                            .checked(self.allow_missing)
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, checked, _, cx| {
                                v.allow_missing = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        Checkbox::new("gerbers")
                            .label("Include KiCad Gerbers")
                            .checked(self.gerbers)
                            .disabled(
                                self.busy
                                    || !self.project.as_ref().is_some_and(|p| {
                                        p.has_placement()
                                            && p.bom.as_ref().is_some_and(|b| {
                                                b.format == chinamaxxbom_core::board::Format::Kicad
                                            })
                                    }),
                            )
                            .on_click(cx.listener(|v, checked, _, cx| {
                                v.gerbers = *checked;
                                cx.notify();
                            })),
                    ),
            );
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_3()
            .overflow_hidden()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Bill of materials"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{assigned} / {count} assigned")),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .items_start()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .child(Table::new(&self.bom_table).small()),
                    )
                    .child(
                        div()
                            .id("component-inspector")
                            .w(px(285.))
                            .flex_shrink_0()
                            .h_full()
                            .min_h_0()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded_md()
                            .bg(cx.theme().background)
                            .child(details)
                            .overflow_y_scrollbar(),
                    ),
            )
            .child(footer)
            .into_any_element()
    }
    fn search_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let part = self
            .catalog_selection
            .and_then(|i| self.results.get(i))
            .cloned();
        let id = part.as_ref().map(|p| p.lcsc.clone()).unwrap_or_default();
        let library_id = id.clone();
        let datasheet = part
            .as_ref()
            .map(|p| p.datasheet.clone())
            .unwrap_or_default();
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
                div().flex_1().min_h_0().overflow_hidden().child(Table::new(&self.catalog_table).small()),
            )
            .child(h_flex().flex_shrink_0().gap_2()
                .child(Button::new("assign-catalog-part").label("Use selected part").primary().disabled(self.busy||self.selected.is_none()||part.is_none()).on_click(cx.listener(move|v,_,w,cx|{v.lcsc.update(cx,|s,cx|s.set_value(id.clone(),w,cx));v.tab=0;v.pick(id.clone(),cx);})))
                .child(Button::new("catalog-import").label("Import library").disabled(self.busy||part.is_none()).on_click(cx.listener(move|v,_,w,cx|{v.import_id.update(cx,|s,cx|s.set_value(library_id.clone(),w,cx));v.bundle.update(cx,|s,cx|s.set_value("",w,cx));v.tab=2;cx.notify();})))
                .child(Button::new("catalog-datasheet").label("Datasheet").disabled(!datasheet.starts_with("https://")).on_click(cx.listener(move|_,_,_,cx|{if datasheet.starts_with("https://"){cx.open_url(&datasheet);}}))))
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
            .overflow_hidden()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Project"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.path).small().disabled(self.busy)),
                    )
                    .child(
                        Button::new("browse-board")
                            .label("Browse…")
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, w, cx| v.browse(0, w, cx))),
                    )
                    .child(
                        Button::new("open-board")
                            .label("Open")
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(|v, _, _, cx| v.open(cx))),
                    )
                    .child(
                        Button::new("theme-toggle")
                            .icon(if cx.theme().is_dark() {
                                IconName::Sun
                            } else {
                                IconName::Moon
                            })
                            .label(if cx.theme().is_dark() {
                                "Light mode"
                            } else {
                                "Dark mode"
                            })
                            .small()
                            .on_click(cx.listener(|_, _, window, cx| {
                                let mode = if cx.theme().is_dark() {
                                    ThemeMode::Light
                                } else {
                                    ThemeMode::Dark
                                };
                                Theme::change(mode, Some(window), cx);
                            })),
                    ),
            )
            .child(
                div().flex_shrink_0().px_4().pt_1().child(
                    TabBar::new("workspace-tabs")
                        .underline()
                        .selected_index(self.tab as usize)
                        .children([
                            Tab::new().label("Bill of materials"),
                            Tab::new().label("JLCPCB catalogue"),
                            Tab::new().label("Library import"),
                        ])
                        .on_click(cx.listener(|v, index, _, cx| {
                            v.tab = *index as u8;
                            cx.notify();
                        })),
                ),
            )
            .children(self.error.as_ref().map(|error| {
                div()
                    .id("operation-error")
                    .flex_shrink_0()
                    .max_h(px(90.))
                    .px_4()
                    .py_2()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .overflow_y_scroll()
                    .child(error.clone())
            }))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .p_4()
                    .child(panel),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .h(px(30.))
                    .px_4()
                    .gap_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .child(div().text_xs().child(if self.busy {
                        "Working…"
                    } else if self.error.is_some() {
                        "Error"
                    } else {
                        "Ready"
                    }))
                    .child(
                        div()
                            .id("status")
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .truncate()
                            .child(self.error.clone().unwrap_or_else(|| self.status.clone()))
                            .text_color(if self.error.is_some() {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            }),
                    ),
            );
        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                gpui_component::TitleBar::new().child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("ChinamaxxBOM"),
                ),
            )
            .child(content)
    }
}
