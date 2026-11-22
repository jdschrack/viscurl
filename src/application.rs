use std::collections::BTreeSet;

use crate::domain::{
    request::{Draft, Field, Request, Response, prepare},
    spec::{Api, Operation},
    variables::Variables,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Endpoints,
    Request,
    Response,
}

impl Pane {
    pub fn step(self, backwards: bool) -> Self {
        match (self, backwards) {
            (Self::Endpoints, false) | (Self::Response, true) => Self::Request,
            (Self::Request, false) | (Self::Endpoints, true) => Self::Response,
            _ => Self::Endpoints,
        }
    }
}

#[derive(Clone, Copy)]
pub enum EndpointRow<'a> {
    Folder {
        tag: &'a str,
        count: usize,
        expanded: bool,
    },
    Operation(usize),
}

#[derive(Clone, Copy)]
pub enum TreeAction {
    Open,
    Expand,
    Collapse,
}

pub struct Exchange {
    pub operation: usize,
    pub result: Result<Response, String>,
}

pub struct App {
    pub api: Api,
    pub pane: Pane,
    pub selected: usize,
    pub tree_cursor: usize,
    pub field: usize,
    pub search: String,
    pub pending: Option<usize>,
    pub message: Option<String>,
    pub recent: Vec<Exchange>,
    pub variables: Variables,
    drafts: Vec<Option<Draft>>,
    matches: Vec<usize>,
    expanded: BTreeSet<String>,
    search_expanded: Option<BTreeSet<String>>,
}

impl App {
    pub fn new(api: Api) -> Self {
        let matches = (0..api.operations.len()).collect();
        let drafts = api.operations.iter().map(|_| None).collect();
        Self {
            api,
            pane: Pane::Endpoints,
            selected: 0,
            tree_cursor: 0,
            field: 0,
            search: String::new(),
            pending: None,
            message: None,
            recent: Vec::new(),
            variables: Variables::default(),
            drafts,
            matches,
            expanded: BTreeSet::new(),
            search_expanded: None,
        }
    }

    pub fn focus(self, pane: Pane) -> Self {
        Self { pane, ..self }
    }

    pub fn select_field(self, field: usize) -> Self {
        Self { field, ..self }
    }

    pub fn with_message(self, message: String) -> Self {
        Self {
            message: Some(message),
            ..self
        }
    }

    pub fn with_variables(self, variables: Variables) -> Self {
        Self {
            variables,
            message: None,
            ..self
        }
    }

    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    pub fn operation(&self) -> &Operation {
        &self.api.operations[self.selected]
    }

    pub fn draft(&self) -> std::borrow::Cow<'_, Draft> {
        self.drafts[self.selected].as_ref().map_or_else(
            || std::borrow::Cow::Owned(Draft::new(self.operation())),
            std::borrow::Cow::Borrowed,
        )
    }

    fn open_folders(&self) -> &BTreeSet<String> {
        self.search_expanded.as_ref().unwrap_or(&self.expanded)
    }

    pub fn tree_rows(&self) -> impl Iterator<Item = EndpointRow<'_>> {
        self.matches
            .chunk_by(|&left, &right| {
                self.api.operations[left].tag == self.api.operations[right].tag
            })
            .flat_map(|operations| {
                let tag = self.api.operations[operations[0]].tag.as_str();
                let expanded = self.open_folders().contains(tag);
                std::iter::once(EndpointRow::Folder {
                    tag,
                    count: operations.len(),
                    expanded,
                })
                .chain(
                    operations
                        .iter()
                        .copied()
                        .take(if expanded { operations.len() } else { 0 })
                        .map(EndpointRow::Operation),
                )
            })
    }

    pub fn filter(self, search: String) -> Self {
        let needle = search.trim().to_lowercase();
        let matches: Vec<_> = self
            .api
            .operations
            .iter()
            .enumerate()
            .filter(|(_, op)| {
                [&op.path, &op.summary, &op.tag, &op.method]
                    .into_iter()
                    .any(|value| value.to_lowercase().contains(&needle))
            })
            .map(|(index, _)| index)
            .collect();
        let selected = if matches.contains(&self.selected) {
            self.selected
        } else {
            matches.first().copied().unwrap_or(self.selected)
        };
        let search_expanded = (!needle.is_empty()).then(|| {
            matches
                .iter()
                .map(|&index| self.api.operations[index].tag.clone())
                .collect()
        });
        let app = Self {
            search,
            matches,
            selected,
            search_expanded,
            field: 0,
            message: None,
            ..self
        };
        let tag = &app.operation().tag;
        let tree_cursor = if app.open_folders().contains(tag) {
            app.tree_rows()
                .position(|row| matches!(row, EndpointRow::Operation(index) if index == selected))
        } else {
            app.parent_row(tag)
        }
        .unwrap_or_default();
        Self { tree_cursor, ..app }
    }

    pub fn can_send(&self) -> bool {
        !self.matches.is_empty()
            && (self.pane != Pane::Endpoints
                || matches!(
                    self.tree_rows().nth(self.tree_cursor),
                    Some(EndpointRow::Operation(_))
                ))
    }

    fn parent_row(&self, parent: &str) -> Option<usize> {
        self.tree_rows()
            .position(|row| matches!(row, EndpointRow::Folder { tag, .. } if tag == parent))
    }

    pub fn move_operation(self, step: isize) -> Self {
        let last = self.tree_rows().count().saturating_sub(1);
        let tree_cursor = self.tree_cursor.saturating_add_signed(step).min(last);
        let endpoint = match self.tree_rows().nth(tree_cursor) {
            Some(EndpointRow::Operation(index)) => index,
            _ => self.selected,
        };
        let field = if endpoint == self.selected {
            self.field
        } else {
            0
        };
        Self {
            tree_cursor,
            selected: endpoint,
            field,
            message: None,
            ..self
        }
    }

    pub fn tree_action(self, action: TreeAction) -> Self {
        let row = self.tree_rows().nth(self.tree_cursor);
        match row {
            Some(EndpointRow::Folder { tag, expanded, .. }) => {
                let open = match action {
                    TreeAction::Open => !expanded,
                    TreeAction::Expand => true,
                    TreeAction::Collapse => false,
                };
                if open == expanded {
                    return self;
                }
                let folders = self
                    .open_folders()
                    .iter()
                    .filter(|name| name.as_str() != tag)
                    .cloned()
                    .chain(open.then(|| tag.to_owned()))
                    .collect();
                if self.search_expanded.is_some() {
                    Self {
                        search_expanded: Some(folders),
                        ..self
                    }
                } else {
                    Self {
                        expanded: folders,
                        ..self
                    }
                }
            }
            Some(EndpointRow::Operation(index))
                if matches!(action, TreeAction::Open | TreeAction::Expand) =>
            {
                let field = if self.selected == index {
                    self.field
                } else {
                    0
                };
                Self {
                    selected: index,
                    field,
                    pane: Pane::Request,
                    ..self
                }
            }
            Some(EndpointRow::Operation(index)) => {
                let tree_cursor = self
                    .parent_row(&self.api.operations[index].tag)
                    .unwrap_or(self.tree_cursor);
                Self {
                    tree_cursor,
                    ..self
                }
            }
            None => self,
        }
    }

    pub fn change_draft(self, change: impl FnOnce(Draft, &Operation) -> Draft) -> Self {
        let selected = self.selected;
        let operation = &self.api.operations[selected];
        let mut change = Some(change);
        let drafts = self
            .drafts
            .into_iter()
            .enumerate()
            .map(|(index, draft)| {
                let Some(change) = change.take_if(|_| index == selected) else {
                    return draft;
                };
                Some(change(
                    draft.unwrap_or_else(|| Draft::new(operation)),
                    operation,
                ))
            })
            .collect();
        Self {
            drafts,
            message: None,
            ..self
        }
    }

    pub fn edit(self, field: Field, value: String) -> Self {
        self.change_draft(|draft, _| draft.edit(field, value))
    }

    pub fn cycle(self, field: Field) -> Self {
        self.change_draft(|draft, operation| draft.cycle(operation, field))
    }

    pub fn start(self) -> (Self, Option<Request>) {
        if self.pending.is_some() {
            return (self, None);
        }
        match prepare(self.operation(), &self.draft(), &self.variables) {
            Ok(request) => (
                Self {
                    pending: Some(self.selected),
                    message: None,
                    pane: Pane::Response,
                    ..self
                },
                Some(request),
            ),
            Err(message) => (
                Self {
                    message: Some(message),
                    pane: Pane::Request,
                    ..self
                },
                None,
            ),
        }
    }

    pub fn finish(self, result: Result<Response, String>) -> Self {
        let Some(operation) = self.pending else {
            return self;
        };
        let recent = std::iter::once(Exchange { operation, result })
            .chain(self.recent)
            .take(8)
            .collect();
        Self {
            recent,
            pending: None,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::spec::parse;

    #[test]
    fn folders_expand_and_collapse_independently_and_restore_after_search() {
        let app = App::new(parse("openapi: 3.1.0\npaths:\n  /a: {get: {tags: [auth]}}\n  /p: {get: {tags: [patients]}}\n  /s: {get: {tags: [status]}}").unwrap());
        assert_eq!(app.tree_rows().count(), 3);
        assert!(!app.can_send());

        let app = app
            .tree_action(TreeAction::Open)
            .move_operation(2)
            .tree_action(TreeAction::Expand);
        assert_eq!(app.tree_rows().count(), 5);
        assert!(matches!(
            app.tree_rows().nth(app.tree_cursor),
            Some(EndpointRow::Folder {
                tag: "patients",
                expanded: true,
                ..
            })
        ));

        let app = app.move_operation(1);
        assert_eq!(app.operation().path, "/p");
        assert!(app.can_send());
        let app = app
            .tree_action(TreeAction::Collapse)
            .tree_action(TreeAction::Collapse);
        assert_eq!(app.tree_rows().count(), 4);
        let app = app.filter("status".into());
        assert!(matches!(
            app.tree_rows().nth(app.tree_cursor),
            Some(EndpointRow::Operation(_))
        ));
        let app = app.filter(String::new());
        assert_eq!(app.tree_rows().count(), 4);
        assert!(matches!(
            app.tree_rows().nth(app.tree_cursor),
            Some(EndpointRow::Folder {
                tag: "status",
                expanded: false,
                ..
            })
        ));
    }

    #[test]
    fn keeps_drafts_when_searching_and_attributes_result_to_original_operation() {
        let app = App::new(parse("openapi: 3.1.0\nservers: [{url: 'http://localhost:8080'}]\npaths: {/a: {get: {}}, /b: {post: {}}}").unwrap());
        let app = app.edit(Field::Headers, "X-Local: one".into());
        let (app, request) = app.start();
        assert_eq!(request.unwrap().url.path(), "/a");
        let app = app.filter("POST".into());
        assert_eq!(app.operation().path, "/b");
        let (app, duplicate) = app.start();
        assert!(duplicate.is_none());
        let app = app.filter("GET".into()).finish(Err("offline".into()));
        assert_eq!(app.draft().headers, "X-Local: one");
        assert_eq!(app.recent[0].operation, 0);
        assert!(app.pending.is_none());
    }
}
