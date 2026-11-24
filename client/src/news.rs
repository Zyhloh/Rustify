use crate::models::Card;
use crate::pages::card_data;
use crate::{App, ReleaseData, Store};
use serde::Deserialize;
use slint::{ComponentHandle, Image, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub card: Card,
    pub date: String,
    pub kind: String,
    pub detail: String,
}

struct NewsState {
    all: RefCell<Vec<ReleaseData>>,
    shown: Rc<VecModel<ReleaseData>>,
}

thread_local! {
    static NEWS: NewsState = NewsState {
        all: RefCell::new(Vec::new()),
        shown: Rc::new(VecModel::default()),
    };
}

pub fn install(app: &App) {
    NEWS.with(|n| app.global::<Store>().set_news(ModelRc::from(n.shown.clone())));
}

pub fn begin(app: &App) {
    let store = app.global::<Store>();
    store.set_news_loading(NEWS.with(|n| n.all.borrow().is_empty()));
}

fn pretty(date: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let parts: Vec<&str> = date.split('-').collect();
    match (parts.first(), parts.get(1).and_then(|m| m.parse::<usize>().ok()), parts.get(2).and_then(|d| d.parse::<u32>().ok())) {
        (Some(year), Some(m @ 1..=12), Some(day)) => format!("{} {day}, {year}", MONTHS[m - 1]),
        _ => date.to_owned(),
    }
}

pub fn set(app: &App, list: &[Release]) -> Vec<String> {
    let mut keys = Vec::new();
    let rows: Vec<ReleaseData> = list
        .iter()
        .map(|r| ReleaseData {
            card: card_data(&r.card, &mut keys),
            date: pretty(&r.date).into(),
            kind: r.kind.as_str().into(),
            detail: r.detail.as_str().into(),
        })
        .collect();
    NEWS.with(|n| *n.all.borrow_mut() = rows);
    filter(app, &app.global::<Store>().get_news_filter());
    app.global::<Store>().set_news_loading(false);
    keys
}

pub fn filter(app: &App, kind: &str) {
    app.global::<Store>().set_news_filter(kind.into());
    NEWS.with(|n| {
        let rows: Vec<ReleaseData> = n.all.borrow().iter().filter(|r| kind.is_empty() || r.kind == kind).cloned().collect();
        n.shown.set_vec(rows);
    });
}

pub fn apply_image(key: &str, image: &Image) {
    NEWS.with(|n| {
        for row in n.all.borrow_mut().iter_mut().filter(|r| r.card.image_url == key) {
            row.card.art = image.clone();
        }
        for i in 0..n.shown.row_count() {
            if let Some(mut row) = n.shown.row_data(i)
                && row.card.image_url == key {
                    row.card.art = image.clone();
                    n.shown.set_row_data(i, row);
                }
        }
    });
}
