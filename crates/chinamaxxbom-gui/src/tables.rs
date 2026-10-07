use chinamaxxbom_core::{board, supplier};
use gpui::*;
use gpui_component::{
    table::{Column, TableDelegate, TableState},
    ActiveTheme,
};

pub struct BomTable {
    columns: Vec<Column>,
    pub rows: Vec<board::Part>,
}
impl BomTable {
    pub fn new() -> Self {
        Self {
            columns: vec![
                Column::new("reference", "Reference")
                    .width(100.)
                    .fixed_left(),
                Column::new("value", "Value").width(150.),
                Column::new("footprint", "Footprint").width(220.),
                Column::new("lcsc", "LCSC part").width(125.),
                Column::new("side", "Side").width(80.),
                Column::new("assembly", "Assembly").width(100.),
            ],
            rows: vec![],
        }
    }
    pub fn set_rows(&mut self, mut rows: Vec<board::Part>) {
        fn reference_key(s: &str) -> (&str, u64, &str) {
            let i = s.find(|c: char| c.is_ascii_digit()).unwrap_or(s.len());
            (&s[..i], s[i..].parse().unwrap_or(0), s)
        }
        rows.sort_by(|a, b| reference_key(&a.reference).cmp(&reference_key(&b.reference)));
        self.rows = rows;
    }
}
impl TableDelegate for BomTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }
    fn column(&self, i: usize, _: &App) -> &Column {
        &self.columns[i]
    }
    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let p = &self.rows[row];
        let value = match col {
            0 => p.reference.clone(),
            1 => p.value.clone(),
            2 => p.footprint.rsplit(':').next().unwrap_or("").into(),
            3 => {
                if p.lcsc.is_empty() {
                    "Unassigned".into()
                } else {
                    p.lcsc.clone()
                }
            }
            4 => if !p.placed {
                "—"
            } else if p.side == board::Side::Top {
                "Top"
            } else {
                "Bottom"
            }
            .into(),
            _ => if p.dnp {
                "DNP"
            } else if p.exclude_bom {
                "Excluded"
            } else {
                "Populate"
            }
            .into(),
        };
        div()
            .truncate()
            .text_color(if col == 3 && p.lcsc.is_empty() {
                cx.theme().muted_foreground
            } else {
                cx.theme().foreground
            })
            .child(value)
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .p_6()
            .text_color(cx.theme().muted_foreground)
            .child("Open a project or schematic to load its bill of materials.")
    }
}

pub struct CatalogTable {
    columns: Vec<Column>,
    pub rows: Vec<supplier::Part>,
}
impl CatalogTable {
    pub fn new() -> Self {
        Self {
            columns: vec![
                Column::new("lcsc", "LCSC part").width(120.).fixed_left(),
                Column::new("model", "Manufacturer part").width(210.),
                Column::new("brand", "Manufacturer").width(140.),
                Column::new("package", "Package").width(120.),
                Column::new("stock", "Stock").width(100.).text_right(),
                Column::new("price", "Unit price").width(100.).text_right(),
                Column::new("library", "Library").width(95.),
                Column::new("description", "Description").width(320.),
            ],
            rows: vec![],
        }
    }
}
impl TableDelegate for CatalogTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }
    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }
    fn column(&self, i: usize, _: &App) -> &Column {
        &self.columns[i]
    }
    fn render_td(
        &mut self,
        row: usize,
        col: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let p = &self.rows[row];
        let value = match col {
            0 => p.lcsc.clone(),
            1 => p.model.clone(),
            2 => p.brand.clone(),
            3 => p.package.clone(),
            4 => p.stock.to_string(),
            5 => p.price.map(|v| format!("{v:.4}")).unwrap_or("—".into()),
            6 => if p.basic { "Basic" } else { "Extended" }.into(),
            _ => p.description.clone(),
        };
        div().truncate().child(value)
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .p_6()
            .text_color(cx.theme().muted_foreground)
            .child("Search JLCPCB by part number, value, or description.")
    }
}
