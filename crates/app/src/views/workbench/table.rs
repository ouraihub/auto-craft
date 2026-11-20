//! Virtual table data, local filtering/sorting and deferred page requests.
use super::{WorkbenchView, data};
use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableState};
use gpui_kit::{
    App, Context, IntoElement, ParentElement as _, SharedString, Styled as _, WeakEntity, Window,
    div,
};
use serde_json::Value;

#[derive(Debug)]
pub(super) struct Rows {
    owner: WeakEntity<WorkbenchView>,
    all: Vec<Value>,
    visible: Vec<usize>,
    filter: String,
    sort: Option<(usize, ColumnSort)>,
    pub next: Option<usize>,
    pub loading: bool,
    pub network: bool,
}
impl Rows {
    pub fn new(owner: WeakEntity<WorkbenchView>) -> Self {
        Self {
            owner,
            all: Vec::new(),
            visible: Vec::new(),
            filter: String::new(),
            sort: None,
            next: None,
            loading: false,
            network: false,
        }
    }
    pub fn clear(&mut self) {
        self.all.clear();
        self.visible.clear();
        self.next = None;
        self.loading = false;
    }
    pub fn append(&mut self, rows: Vec<Value>, next: Option<usize>) {
        self.all.extend(rows);
        self.next = next;
        self.loading = false;
        self.reindex();
    }
    pub fn filter(&mut self, filter: &str) {
        self.filter = filter.to_lowercase();
        self.reindex();
    }
    pub fn row(&self, index: usize) -> Option<&Value> {
        self.visible
            .get(index)
            .and_then(|index| self.all.get(*index))
    }
    fn cell(&self, row: &Value, column: usize) -> String {
        if self.network {
            return data::field(
                row,
                &[match column {
                    0 => "method",
                    1 => "url",
                    _ => "status",
                }],
            )
            .unwrap_or_default();
        }
        match column {
            0 => data::address(row),
            1 => data::name(row),
            _ => data::field(row, &["size"]),
        }
        .unwrap_or_default()
    }
    fn reindex(&mut self) {
        self.visible = (0..self.all.len())
            .filter(|index| {
                let row = &self.all[*index];
                self.filter.is_empty()
                    || self.cell(row, 0).to_lowercase().contains(&self.filter)
                    || self.cell(row, 1).to_lowercase().contains(&self.filter)
            })
            .collect();
        if let Some((column, direction)) = self.sort {
            let mut visible = std::mem::take(&mut self.visible);
            visible.sort_by(|left, right| {
                let a = self.cell(&self.all[*left], column);
                let b = self.cell(&self.all[*right], column);
                let numeric = |s: &str| {
                    s.strip_prefix("0x")
                        .and_then(|s| u64::from_str_radix(s, 16).ok())
                        .or_else(|| s.parse::<u64>().ok())
                };
                let order = match (numeric(&a), numeric(&b)) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    _ => a.cmp(&b),
                };
                if direction == ColumnSort::Descending {
                    order.reverse()
                } else {
                    order
                }
            });
            self.visible = visible;
        }
    }
}
impl TableDelegate for Rows {
    fn columns_count(&self, _: &App) -> usize {
        3
    }
    fn rows_count(&self, _: &App) -> usize {
        self.visible.len()
    }
    fn column(&self, index: usize, _: &App) -> Column {
        let key = if self.network {
            ["method", "url", "status"][index]
        } else {
            ["address", "name", "size"][index]
        };
        Column::new(key, rust_i18n::t!(format!("app.workbench.{key}"))).sortable()
    }
    fn perform_sort(
        &mut self,
        index: usize,
        sort: ColumnSort,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        self.sort = match sort {
            ColumnSort::Default => None,
            ColumnSort::Ascending | ColumnSort::Descending => Some((index, sort)),
        };
        self.reindex();
        cx.notify();
    }
    fn render_td(
        &mut self,
        row: usize,
        column: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div().px_2().child(
            self.row(row)
                .map(|row| self.cell(row, column))
                .unwrap_or_default(),
        )
    }
    fn cell_label(&self, row: usize, column: usize, _: &App) -> Option<SharedString> {
        self.row(row).map(|row| self.cell(row, column).into())
    }
    fn has_more(&self, _: &App) -> bool {
        self.next.is_some() && !self.loading
    }
    fn load_more(&mut self, _: &mut Window, cx: &mut Context<TableState<Self>>) {
        if self.loading || self.next.is_none() {
            return;
        }
        self.loading = true;
        let owner = self.owner.clone();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |this, cx| this.load_list(false, cx));
        });
    }
}
