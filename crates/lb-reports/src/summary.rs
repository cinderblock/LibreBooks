//! The machinery shared by summary reports (P&L, balance sheet, trial balance,
//! cash flows): columns that are date ranges and dimensions, comparison and
//! percentage columns derived from them, and accounts rendered as QBD indents
//! them.
//!
//! A *slot* is one date range and one dimension value ("October, job Phase 1").
//! A value column sums one or more slots ("Total Acme" sums its jobs' slots).
//! Slots that share a comparison set are computed with one SQL query over their
//! combined range, grouped by month (or by day when a boundary falls mid-month)
//! and by the dimension, then distributed; so twelve monthly columns cost one
//! query, not twelve.

use std::collections::{BTreeMap, HashMap, HashSet};

use lb_core::{AccountId, ClassId, DateRange, EntityId, ItemId, Money};
use rust_decimal::{Decimal, RoundingStrategy};

use crate::Result;
use crate::ctx::Ctx;
use crate::dates;
use crate::ledger::{Dims, Gran, Ledger};
use crate::model::{Cell, Column, ColumnKind, Drill, Row, RowKind};
use crate::request::{ColumnLayout, ReportRequest};

/// Which slice of a dimension a slot sums.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DimSel {
    Any,
    /// The customer or job (`None`: lines with no customer:job).
    Job(Option<EntityId>),
    Class(Option<ClassId>),
    Vendor(Option<EntityId>),
    Item(Option<ItemId>),
}

impl DimSel {
    fn dims(self) -> Dims {
        match self {
            DimSel::Any => Dims::default(),
            DimSel::Job(_) | DimSel::Vendor(_) => Dims {
                names: true,
                ..Dims::default()
            },
            DimSel::Class(_) => Dims {
                class: true,
                ..Dims::default()
            },
            DimSel::Item(_) => Dims {
                item: true,
                ..Dims::default()
            },
        }
    }

    fn matches(
        self,
        ctx: &Ctx,
        entity: Option<EntityId>,
        txn_entity: Option<EntityId>,
        class: Option<ClassId>,
        item: Option<ItemId>,
    ) -> bool {
        match self {
            DimSel::Any => true,
            DimSel::Job(k) => ctx.job_of(entity, txn_entity) == k,
            DimSel::Vendor(k) => ctx.vendor_of(entity, txn_entity) == k,
            DimSel::Class(k) => class == k,
            DimSel::Item(k) => item == k,
        }
    }
}

/// One date range and dimension value; `set` groups slots computed together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub range: DateRange,
    pub dim: DimSel,
    pub set: u8,
}

fn union(a: DateRange, b: DateRange) -> DateRange {
    DateRange {
        from: a.from.and_then(|x| b.from.map(|y| x.min(y))),
        to: a.to.and_then(|x| b.to.map(|y| x.max(y))),
    }
}

/// Debit-positive sums per account for each slot.
pub fn compute_slots(ledger: &Ledger, slots: &[Slot]) -> Result<Vec<HashMap<AccountId, Money>>> {
    let mut out: Vec<HashMap<AccountId, Money>> = vec![HashMap::new(); slots.len()];
    let mut sets: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    for (i, s) in slots.iter().enumerate() {
        sets.entry(s.set).or_default().push(i);
    }
    for idx in sets.values() {
        let envelope = idx
            .iter()
            .map(|&i| slots[i].range)
            .reduce(union)
            .unwrap_or(DateRange::ALL);
        let distinct_ranges: HashSet<(Option<_>, Option<_>)> = idx
            .iter()
            .map(|&i| (slots[i].range.from, slots[i].range.to))
            .collect();
        let gran = if distinct_ranges.len() <= 1 {
            Gran::None
        } else if idx.iter().all(|&i| {
            let r = slots[i].range;
            r.from
                .is_none_or(|f| Some(f) == envelope.from || f.day() == 1)
                && r.to
                    .is_none_or(|t| Some(t) == envelope.to || t == t.last_of_month())
        }) {
            Gran::Month
        } else {
            Gran::Day
        };
        let mut dims = Dims::default();
        for &i in idx {
            let d = slots[i].dim.dims();
            dims.names |= d.names;
            dims.class |= d.class;
            dims.item |= d.item;
        }
        let agg = ledger.aggregate(envelope, gran, dims)?;
        for (k, v) in agg {
            let span = gran.span(&k.bucket);
            for &i in idx {
                let s = &slots[i];
                let in_range = match span {
                    None => true,
                    Some((first, last)) => {
                        s.range.from.is_none_or(|f| f <= last)
                            && s.range.to.is_none_or(|t| t >= first)
                    }
                };
                if in_range
                    && s.dim.matches(
                        ledger.ctx,
                        k.entity_id,
                        k.txn_entity_id,
                        k.class_id,
                        k.item_id,
                    )
                {
                    *out[i].entry(k.account_id).or_default() += v;
                }
            }
        }
    }
    Ok(out)
}

/// A column of amounts: the sum of some slots.
#[derive(Clone, Debug)]
pub struct ValueCol {
    pub label: String,
    pub slots: Vec<usize>,
    /// For QuickZoom: the range and dimension the column covers.
    pub range: DateRange,
    pub dim: DimSel,
}

/// A displayed column after the label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutCol {
    Value(usize),
    /// This value column as a percent of its base (income, total assets…).
    PctOf(usize),
    /// First minus second.
    Change(usize, usize),
    /// (First minus second) as a percent of the second.
    PctChange(usize, usize),
}

/// A base column before comparisons: a label, a range, and the dimension values
/// it sums.
#[derive(Clone, Debug)]
pub struct BaseCol {
    pub label: String,
    pub range: DateRange,
    pub dims: Vec<DimSel>,
    /// The dimension shown for QuickZoom (a single job, class…), if one.
    pub drill_dim: DimSel,
}

impl BaseCol {
    pub fn total(label: impl Into<String>, range: DateRange) -> BaseCol {
        BaseCol {
            label: label.into(),
            range,
            dims: vec![DimSel::Any],
            drill_dim: DimSel::Any,
        }
    }
}

/// Which derived columns to add.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlanOpts {
    /// "% of Income" or "% of Column", if wanted.
    pub pct_label: Option<&'static str>,
    pub previous_period: bool,
    pub previous_year: bool,
    pub dollar_change: bool,
    pub percent_change: bool,
}

impl PlanOpts {
    pub fn from_request(req: &ReportRequest, pct_label: &'static str, pct: bool) -> PlanOpts {
        PlanOpts {
            pct_label: pct.then_some(pct_label),
            previous_period: req.previous_period,
            previous_year: req.previous_year,
            dollar_change: req.dollar_change,
            percent_change: req.percent_change,
        }
    }
}

/// The columns of a summary report and how to compute them.
#[derive(Clone, Debug)]
pub struct Plan {
    pub slots: Vec<Slot>,
    pub values: Vec<ValueCol>,
    pub out: Vec<(OutCol, Column)>,
    /// Per value column: what "% of" divides by. Set by the report once known.
    pub bases: Vec<Money>,
    /// The first column's heading.
    pub label_heading: String,
}

impl Plan {
    /// Columns for `bases` and, after each, its percent, comparison and change
    /// columns as `opts` asks.
    pub fn new(bases: &[BaseCol], opts: PlanOpts) -> Plan {
        let mut plan = Plan {
            slots: Vec::new(),
            values: Vec::new(),
            out: Vec::new(),
            bases: Vec::new(),
            label_heading: String::new(),
        };
        let grouped = bases.len() > 1 && (opts.previous_period || opts.previous_year);
        for (bi, b) in bases.iter().enumerate() {
            let group = grouped.then(|| b.label.clone());
            let cur = plan.add_value(b.label.clone(), b.range, &b.dims, b.drill_dim, 0);
            plan.push_out(OutCol::Value(cur), ColumnKind::Money, group.as_deref());
            if let Some(p) = opts.pct_label {
                plan.out.push((
                    OutCol::PctOf(cur),
                    Column::new(format!("p{bi}"), p, ColumnKind::Percent).grouped(group.clone()),
                ));
            }
            for (set, wanted, shift) in [
                (
                    1u8,
                    opts.previous_period,
                    dates::previous_period as fn(DateRange) -> DateRange,
                ),
                (2u8, opts.previous_year, dates::previous_year),
            ] {
                if !wanted {
                    continue;
                }
                let range = shift(b.range);
                let label = if matches!(
                    b.range,
                    DateRange {
                        from: None,
                        to: None
                    }
                ) {
                    format!("{} (prior)", b.label)
                } else {
                    dates::short_range(range)
                };
                let prev = plan.add_value(label, range, &b.dims, b.drill_dim, set);
                plan.push_out(OutCol::Value(prev), ColumnKind::Money, group.as_deref());
                if let Some(p) = opts.pct_label {
                    plan.out.push((
                        OutCol::PctOf(prev),
                        Column::new(format!("p{bi}_{set}"), p, ColumnKind::Percent)
                            .grouped(group.clone()),
                    ));
                }
                if opts.dollar_change {
                    plan.out.push((
                        OutCol::Change(cur, prev),
                        Column::new(format!("d{bi}_{set}"), "$ Change", ColumnKind::Money)
                            .grouped(group.clone()),
                    ));
                }
                if opts.percent_change {
                    plan.out.push((
                        OutCol::PctChange(cur, prev),
                        Column::new(format!("x{bi}_{set}"), "% Change", ColumnKind::Percent)
                            .grouped(group.clone()),
                    ));
                }
            }
        }
        plan.bases = vec![Money::ZERO; plan.values.len()];
        plan
    }

    fn add_value(
        &mut self,
        label: String,
        range: DateRange,
        dims: &[DimSel],
        drill_dim: DimSel,
        set: u8,
    ) -> usize {
        let mut slots = Vec::new();
        for d in dims {
            slots.push(self.slots.len());
            self.slots.push(Slot {
                range,
                dim: *d,
                set,
            });
        }
        self.values.push(ValueCol {
            label,
            slots,
            range,
            dim: drill_dim,
        });
        self.values.len() - 1
    }

    fn push_out(&mut self, col: OutCol, kind: ColumnKind, group: Option<&str>) {
        let OutCol::Value(v) = col else {
            return;
        };
        let id = format!("c{v}");
        self.out.push((
            col,
            Column::new(id, self.values[v].label.clone(), kind).grouped(group.map(str::to_owned)),
        ));
    }

    /// The report's columns: the label column, then the planned ones.
    pub fn columns(&self) -> Vec<Column> {
        let mut cols = vec![Column::new(
            "label",
            self.label_heading.clone(),
            ColumnKind::Text,
        )];
        cols.extend(self.out.iter().map(|(_, c)| c.clone()));
        cols
    }

    /// Debit-positive sums per account for each value column.
    pub fn compute(&self, ledger: &Ledger) -> Result<Vec<HashMap<AccountId, Money>>> {
        let slot_values = compute_slots(ledger, &self.slots)?;
        Ok(self
            .values
            .iter()
            .map(|v| {
                let mut m: HashMap<AccountId, Money> = HashMap::new();
                for &s in &v.slots {
                    for (a, x) in &slot_values[s] {
                        *m.entry(*a).or_default() += *x;
                    }
                }
                m
            })
            .collect())
    }

    /// The cells after the label for a line's values (one per value column).
    pub fn cells(&self, values: &[Money], drills: &[Option<Drill>]) -> Vec<Cell> {
        self.out
            .iter()
            .map(|(c, _)| match *c {
                OutCol::Value(v) => Cell::money(values[v]).drill(drills.get(v).cloned().flatten()),
                OutCol::PctOf(v) => Cell::percent(percent(values[v], self.bases[v])),
                OutCol::Change(a, b) => Cell::money(values[a] - values[b]),
                OutCol::PctChange(a, b) => {
                    Cell::percent(percent(values[a] - values[b], values[b].abs()))
                }
            })
            .collect()
    }

    /// Render lines as rows.
    pub fn rows(&self, lines: Vec<Line>) -> Vec<Row> {
        let width = self.out.len() + 1;
        lines
            .into_iter()
            .map(|l| {
                if l.kind == RowKind::Spacer {
                    return Row::spacer(width);
                }
                let mut cells = vec![Cell::text(l.label)];
                match &l.values {
                    Some(v) => cells.extend(self.cells(v, &l.value_drills)),
                    None => cells.extend(std::iter::repeat_n(Cell::EMPTY, self.out.len())),
                }
                Row::new(l.kind, l.depth, cells).drill(l.drill)
            })
            .collect()
    }
}

/// `part / whole` as a percent with one decimal place, or nothing when `whole`
/// is zero.
pub fn percent(part: Money, whole: Money) -> Option<String> {
    if whole.is_zero() {
        return None;
    }
    let p = (Decimal::from(part.0) * Decimal::ONE_HUNDRED / Decimal::from(whole.0))
        .round_dp_with_strategy(1, RoundingStrategy::MidpointAwayFromZero);
    Some(format!("{p:.1}"))
}

/// A row before it is laid out in columns.
#[derive(Clone, Debug)]
pub struct Line {
    pub kind: RowKind,
    pub depth: u32,
    pub label: String,
    /// One per value column; `None` for headers.
    pub values: Option<Vec<Money>>,
    pub drill: Option<Drill>,
    pub value_drills: Vec<Option<Drill>>,
}

impl Line {
    pub fn header(depth: u32, label: impl Into<String>) -> Line {
        Line {
            kind: RowKind::Header,
            depth,
            label: label.into(),
            values: None,
            drill: None,
            value_drills: Vec::new(),
        }
    }

    pub fn amounts(
        kind: RowKind,
        depth: u32,
        label: impl Into<String>,
        values: Vec<Money>,
    ) -> Line {
        Line {
            kind,
            depth,
            label: label.into(),
            values: Some(values),
            drill: None,
            value_drills: Vec::new(),
        }
    }
}

/// Element-wise sum.
pub fn add(a: &mut [Money], b: &[Money]) {
    for (x, y) in a.iter_mut().zip(b) {
        *x += *y;
    }
}

/// Element-wise difference `a - b`.
pub fn sub(a: &[Money], b: &[Money]) -> Vec<Money> {
    a.iter().zip(b).map(|(x, y)| *x - *y).collect()
}

/// QuickZoom for one amount: what to open, given the row's key, whether to leave
/// out what is below it, and the value column (`None`: the row itself).
pub type DrillFn<'a, K> = dyn Fn(K, bool, Option<usize>) -> Option<Drill> + 'a;

/// A hierarchy to render (accounts and subaccounts, customers and jobs, items).
pub struct Tree<'a, K> {
    /// A node's children, in display order.
    pub children: &'a dyn Fn(K) -> Vec<K>,
    pub label: &'a dyn Fn(K) -> String,
    /// A node's own amounts (one per value column), in the sign the report shows.
    pub values: &'a dyn Fn(K) -> Vec<Money>,
    pub drill: &'a DrillFn<'a, K>,
    /// Show this node even when its whole subtree is zero ("show zero rows").
    pub keep_zero: &'a dyn Fn(K) -> bool,
    pub width: usize,
}

/// Lines as QBD indents a hierarchy: a parent with children gets a heading, the
/// children indented below, its own amounts as "<parent> - Other", and a
/// "Total <parent>" line; a childless node is one line. Nodes whose whole
/// subtree is zero are left out unless `keep_zero`. Returns the lines and the
/// total per value column.
pub fn tree_lines<K: Copy>(t: &Tree<'_, K>, roots: &[K], depth: u32) -> (Vec<Line>, Vec<Money>) {
    let mut lines = Vec::new();
    let mut total = vec![Money::ZERO; t.width];
    for r in roots {
        if let Some((l, s)) = render_node(t, *r, depth) {
            lines.extend(l);
            add(&mut total, &s);
        }
    }
    (lines, total)
}

fn render_node<K: Copy>(t: &Tree<'_, K>, id: K, depth: u32) -> Option<(Vec<Line>, Vec<Money>)> {
    let own = (t.values)(id);
    let mut child_lines = Vec::new();
    let mut total = own.clone();
    for k in (t.children)(id) {
        if let Some((l, s)) = render_node(t, k, depth + 1) {
            child_lines.extend(l);
            add(&mut total, &s);
        }
    }
    let own_nonzero = own.iter().any(|m| !m.is_zero());
    let keep = (t.keep_zero)(id);
    if child_lines.is_empty() && !own_nonzero && !keep {
        return None;
    }
    let label = (t.label)(id);
    let cell_drills = |exact: bool| {
        (0..t.width)
            .map(|v| (t.drill)(id, exact, Some(v)))
            .collect()
    };
    let mut lines = Vec::new();
    if child_lines.is_empty() {
        let mut l = Line::amounts(RowKind::Data, depth, label, own);
        l.drill = (t.drill)(id, false, None);
        l.value_drills = cell_drills(false);
        lines.push(l);
    } else {
        let mut h = Line::header(depth, label.clone());
        h.drill = (t.drill)(id, false, None);
        lines.push(h);
        lines.extend(child_lines);
        if own_nonzero || keep {
            let mut o = Line::amounts(RowKind::Data, depth + 1, format!("{label} - Other"), own);
            o.drill = (t.drill)(id, true, None);
            o.value_drills = cell_drills(true);
            lines.push(o);
        }
        let mut s = Line::amounts(
            RowKind::Subtotal,
            depth,
            format!("Total {label}"),
            total.clone(),
        );
        s.drill = (t.drill)(id, false, None);
        s.value_drills = cell_drills(false);
        lines.push(s);
    }
    Some((lines, total))
}

/// [`tree_lines`] over accounts: `members` (any order) indented under their
/// parents in chart order, labelled as the chart labels them.
pub fn account_tree(
    ctx: &Ctx,
    members: &[AccountId],
    values: &dyn Fn(AccountId) -> Vec<Money>,
    width: usize,
    show_zero: bool,
    depth: u32,
    drill: &DrillFn<'_, AccountId>,
) -> (Vec<Line>, Vec<Money>) {
    let set: HashSet<AccountId> = members.iter().copied().collect();
    let mut roots: Vec<AccountId> = members
        .iter()
        .copied()
        .filter(|a| {
            ctx.account(*a)
                .and_then(|x| x.parent_id)
                .is_none_or(|p| !set.contains(&p))
        })
        .collect();
    roots.sort_by_key(|a| ctx.account_order(*a));
    let children = |id: AccountId| {
        let mut kids: Vec<AccountId> = ctx
            .account_children(id)
            .iter()
            .copied()
            .filter(|c| set.contains(c))
            .collect();
        kids.sort_by_key(|a| ctx.account_order(*a));
        kids
    };
    let label = |id: AccountId| ctx.account_label(id);
    let keep = |id: AccountId| show_zero && ctx.account(id).is_some_and(|a| a.is_active);
    let tree = Tree {
        children: &children,
        label: &label,
        values,
        drill,
        keep_zero: &keep,
        width,
    };
    tree_lines(&tree, &roots, depth)
}

/// [`tree_lines`] over customers and jobs (or any names): every name in
/// `members` plus the customers above them, jobs under their customers, in name
/// order.
pub fn entity_tree(
    ctx: &Ctx,
    members: &HashSet<EntityId>,
    values: &dyn Fn(EntityId) -> Vec<Money>,
    width: usize,
    show_zero: bool,
    depth: u32,
    drill: &DrillFn<'_, EntityId>,
) -> (Vec<Line>, Vec<Money>) {
    let mut all: HashSet<EntityId> = HashSet::new();
    for m in members {
        let mut cur = Some(*m);
        while let Some(c) = cur {
            if !all.insert(c) {
                break;
            }
            cur = ctx.entity(c).and_then(|e| e.parent_id);
        }
    }
    let name = |e: EntityId| {
        ctx.entity(e)
            .map(|x| x.full_name.to_lowercase())
            .unwrap_or_default()
    };
    let mut roots: Vec<EntityId> = all
        .iter()
        .copied()
        .filter(|e| ctx.entity(*e).and_then(|x| x.parent_id).is_none())
        .collect();
    roots.sort_by_key(|e| name(*e));
    let children = |id: EntityId| {
        let mut kids: Vec<EntityId> = ctx
            .entity_children(id)
            .iter()
            .copied()
            .filter(|c| all.contains(c))
            .collect();
        kids.sort_by_key(|e| name(*e));
        kids
    };
    let label = |id: EntityId| {
        ctx.entity(id)
            .map_or_else(|| "(unknown name)".to_owned(), |e| e.name.clone())
    };
    let keep = |id: EntityId| show_zero && members.contains(&id);
    let tree = Tree {
        children: &children,
        label: &label,
        values,
        drill,
        keep_zero: &keep,
        width,
    };
    tree_lines(&tree, &roots, depth)
}

/// The base columns for a layout over `range`: one per period (plus TOTAL), one
/// per name/class/item with amounts (plus totals for parents, and TOTAL), or one
/// for the whole range.
pub fn base_columns(
    ledger: &Ledger,
    range: DateRange,
    layout: ColumnLayout,
    accounts: &HashSet<AccountId>,
) -> Result<Vec<BaseCol>> {
    let ctx = ledger.ctx;
    let total = BaseCol::total(
        if layout == ColumnLayout::TotalOnly {
            dates::short_range(range)
        } else {
            "TOTAL".to_owned()
        },
        range,
    );
    match layout {
        ColumnLayout::TotalOnly => Ok(vec![total]),
        ColumnLayout::Week | ColumnLayout::Month | ColumnLayout::Quarter | ColumnLayout::Year => {
            let Some(to) = range.to.or(Some(ctx.today)) else {
                return Ok(vec![total]);
            };
            let from = match range.from {
                Some(f) => f,
                None => ledger.first_date()?.unwrap_or(to).min(to),
            };
            let mut cols: Vec<BaseCol> = dates::buckets(from, to, layout, ctx.fiscal_start())
                .into_iter()
                .map(|r| BaseCol::total(dates::short_range(r), r))
                .collect();
            cols.push(total);
            Ok(cols)
        }
        ColumnLayout::CustomerJob
        | ColumnLayout::Class
        | ColumnLayout::Vendor
        | ColumnLayout::Item => {
            let dims = match layout {
                ColumnLayout::Class => Dims {
                    class: true,
                    ..Dims::default()
                },
                ColumnLayout::Item => Dims {
                    item: true,
                    ..Dims::default()
                },
                _ => Dims {
                    names: true,
                    ..Dims::default()
                },
            };
            let agg = ledger.aggregate(range, Gran::None, dims)?;
            let mut keys: HashMap<DimSel, Money> = HashMap::new();
            for (k, v) in agg {
                if !accounts.contains(&k.account_id) {
                    continue;
                }
                let sel = match layout {
                    ColumnLayout::CustomerJob => {
                        DimSel::Job(ctx.job_of(k.entity_id, k.txn_entity_id))
                    }
                    ColumnLayout::Vendor => {
                        DimSel::Vendor(ctx.vendor_of(k.entity_id, k.txn_entity_id))
                    }
                    ColumnLayout::Class => DimSel::Class(k.class_id),
                    _ => DimSel::Item(k.item_id),
                };
                *keys.entry(sel).or_default() += v;
            }
            keys.retain(|_, v| !v.is_zero());
            let mut cols = dim_columns(ctx, layout, range, &keys);
            cols.push(total);
            Ok(cols)
        }
    }
}

/// Columns for the dimension values present, in list order, with a total column
/// after each parent's subtree.
fn dim_columns(
    ctx: &Ctx,
    layout: ColumnLayout,
    range: DateRange,
    keys: &HashMap<DimSel, Money>,
) -> Vec<BaseCol> {
    // (sort name, parent chain from the root, the selector) for each present key.
    struct Node {
        name: String,
        label: String,
        parent: Option<DimSel>,
    }
    let node = |d: DimSel| -> Option<Node> {
        match d {
            DimSel::Job(Some(e)) | DimSel::Vendor(Some(e)) => ctx.entity(e).map(|x| Node {
                name: x.full_name.clone(),
                label: x.name.clone(),
                parent: (layout == ColumnLayout::CustomerJob)
                    .then_some(x.parent_id.map(|p| DimSel::Job(Some(p))))
                    .flatten(),
            }),
            DimSel::Class(Some(c)) => ctx.classes.get(&c).map(|x| Node {
                name: x.full_name.clone(),
                label: x.name.clone(),
                parent: x.parent_id.map(|p| DimSel::Class(Some(p))),
            }),
            DimSel::Item(Some(i)) => ctx.item(i).map(|x| Node {
                name: x.full_name.clone(),
                label: x.name.clone(),
                parent: x.parent_id.map(|p| DimSel::Item(Some(p))),
            }),
            _ => None,
        }
    };
    // Every present key plus its ancestors.
    let mut all: HashMap<DimSel, Node> = HashMap::new();
    for k in keys.keys() {
        let mut cur = Some(*k);
        while let Some(c) = cur {
            if all.contains_key(&c) {
                break;
            }
            let Some(n) = node(c) else {
                break;
            };
            cur = n.parent;
            all.insert(c, n);
        }
    }
    let mut children: HashMap<Option<DimSel>, Vec<DimSel>> = HashMap::new();
    for (k, n) in &all {
        children.entry(n.parent).or_default().push(*k);
    }
    for v in children.values_mut() {
        v.sort_by(|a, b| all[a].name.to_lowercase().cmp(&all[b].name.to_lowercase()));
    }
    fn walk(
        k: DimSel,
        all: &HashMap<DimSel, Node>,
        children: &HashMap<Option<DimSel>, Vec<DimSel>>,
        keys: &HashMap<DimSel, Money>,
        range: DateRange,
        out: &mut Vec<BaseCol>,
    ) -> Vec<DimSel> {
        let kids = children.get(&Some(k)).cloned().unwrap_or_default();
        let label = all[&k].label.clone();
        if kids.is_empty() {
            out.push(BaseCol {
                label,
                range,
                dims: vec![k],
                drill_dim: k,
            });
            return vec![k];
        }
        let mut subtree = Vec::new();
        for c in kids {
            subtree.extend(walk(c, all, children, keys, range, out));
        }
        if keys.contains_key(&k) {
            out.push(BaseCol {
                label: format!("{label} - Other"),
                range,
                dims: vec![k],
                drill_dim: k,
            });
        }
        subtree.push(k);
        out.push(BaseCol {
            label: format!("Total {label}"),
            range,
            dims: subtree.clone(),
            drill_dim: k,
        });
        subtree
    }
    let mut out = Vec::new();
    for r in children.get(&None).cloned().unwrap_or_default() {
        walk(r, &all, &children, keys, range, &mut out);
    }
    let none = match layout {
        ColumnLayout::CustomerJob => (DimSel::Job(None), "No Customer:Job"),
        ColumnLayout::Class => (DimSel::Class(None), "Unclassified"),
        ColumnLayout::Vendor => (DimSel::Vendor(None), "No Vendor"),
        _ => (DimSel::Item(None), "No Item"),
    };
    if keys.contains_key(&none.0) {
        out.push(BaseCol {
            label: none.1.to_owned(),
            range,
            dims: vec![none.0],
            drill_dim: none.0,
        });
    }
    out
}
