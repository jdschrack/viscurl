use super::{Edit, Overlay, ResponseTab, Screen, fields, multiline};
use crate::{
    application::{App, EndpointRow, Pane},
    domain::{
        request::{Field, Response, destination, documented},
        spec::{AuthKind, Encoding},
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize as _},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Row, Table, Wrap},
};
use std::borrow::Cow;

const BACKGROUND: Color = Color::Rgb(19, 22, 24);
const PANEL: Color = Color::Rgb(27, 32, 34);
const SELECTED: Color = Color::Rgb(52, 62, 59);
const TEXT: Color = Color::Rgb(232, 239, 249);
const MUTED: Color = Color::Rgb(157, 176, 199);
const BORDER: Color = Color::Rgb(71, 91, 116);
pub(super) const ACCENT: Color = Color::Rgb(214, 232, 154);
const GOOD: Color = Color::Rgb(138, 228, 158);
const WARN: Color = Color::Rgb(249, 202, 124);
const BAD: Color = Color::Rgb(255, 153, 161);
const BLUE: Color = Color::Rgb(141, 192, 255);
const PURPLE: Color = Color::Rgb(194, 169, 255);

pub(super) fn safe(value: &str) -> String {
    value.chars().map(safe_char).collect()
}

pub(super) fn safe_char(ch: char) -> char {
    match ch {
        '\t' => ' ',
        ch if ch.is_control()
            || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' | '\u{061c}') =>
        {
            '�'
        }
        ch => ch,
    }
}

fn panel(title: impl Into<Line<'static>>, active: bool) -> Block<'static> {
    let border = if active { ACCENT } else { BORDER };
    Block::default()
        .borders(Borders::TOP)
        .title(title.into().fg(if active { ACCENT } else { MUTED }).bold())
        .border_style(Style::new().fg(border))
        .style(Style::new().fg(TEXT).bg(PANEL))
        .padding(Padding::horizontal(1))
}

fn method(method: &str) -> Span<'static> {
    let color = match method {
        "GET" => BLUE,
        "POST" => GOOD,
        "PUT" | "PATCH" => WARN,
        "DELETE" => BAD,
        _ => PURPLE,
    };
    Span::styled(method.to_owned(), Style::new().fg(color).bold())
}

fn window(total: usize, selection: usize, visible: usize) -> usize {
    selection
        .saturating_sub(visible / 2)
        .min(total.saturating_sub(visible))
}

fn focused(line: Line<'static>, selected: bool, active: bool) -> Line<'static> {
    if selected {
        return line.bg(SELECTED).add_modifier(if active {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });
    }
    line
}

fn endpoints(frame: &mut Frame<'_>, rect: Rect, app: &App) {
    let active = app.pane == Pane::Endpoints;
    let block = panel(format!(" Endpoints · {} ", app.matches().len()), active);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let find = if app.search.is_empty() {
        "/  Search endpoints".into()
    } else {
        format!("/  {}", safe(&app.search))
    };
    let head = Line::styled(find, Style::new().fg(MUTED));
    let rows = app.tree_rows();
    let limit = usize::from(inner.height.saturating_sub(1));
    let start = window(app.tree_rows().count(), app.tree_cursor, limit);
    let lines = rows
        .enumerate()
        .skip(start)
        .take(limit)
        .map(|(index, row)| {
            let line = match row {
                EndpointRow::Folder {
                    tag,
                    count,
                    expanded,
                } => Line::from(vec![
                    Span::styled(if expanded { "▾ " } else { "▸ " }, Style::new().fg(ACCENT)),
                    Span::styled(safe(tag), Style::new().fg(TEXT).bold()),
                    Span::styled(format!("  {count}"), Style::new().fg(MUTED)),
                ]),
                EndpointRow::Operation(id) => {
                    let op = &app.api.operations[id];
                    Line::from(vec![
                        Span::styled("  │ ", Style::new().fg(BORDER)),
                        method(&op.method),
                        Span::raw(" "),
                        Span::raw(safe(&op.path)),
                    ])
                }
            };
            focused(line, index == app.tree_cursor, active)
        });
    frame.render_widget(
        Paragraph::new(Text::from_iter(std::iter::once(head).chain(lines))),
        inner,
    );
}

fn field_label(app: &App, field: Field) -> String {
    let op = app.operation();
    match field {
        Field::Server => "Server".to_owned(),
        Field::Parameter(index) => {
            let param = &op.parameters[index];
            format!(
                "{} · {}{}{}",
                param.location,
                param.name,
                if param.required { " *" } else { "" },
                if matches!(param.encoding, Encoding::Manual) {
                    " †"
                } else {
                    ""
                }
            )
        }
        Field::Query => "Extra query".to_owned(),
        Field::Headers => "Headers".to_owned(),
        Field::Media => "Content type".to_owned(),
        Field::Body => "Body".to_owned(),
        Field::Auth => "Auth".to_owned(),
        Field::Secret => {
            let draft = app.draft();
            match draft.auth_option(op).map(|option| option.kind) {
                Some(AuthKind::Basic) => "Username:password",
                Some(AuthKind::Bearer) => "Bearer token",
                _ => "API key",
            }
            .into()
        }
    }
}

fn request(frame: &mut Frame<'_>, rect: Rect, app: &App) {
    let active = app.pane == Pane::Request;
    let op = app.operation();
    let draft = app.draft();
    let block = panel(" Request ", active);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if app.matches().is_empty() {
        frame.render_widget(Paragraph::new("No matching endpoints").fg(MUTED), inner);
        return;
    }
    let heading = Line::from(vec![
        method(&op.method),
        Span::raw("  "),
        Span::raw(safe(&op.path)).bold(),
    ]);
    let summary =
        (!op.summary.is_empty()).then(|| Line::styled(safe(&op.summary), Style::new().fg(MUTED)));
    let options = fields(app);
    let limit = usize::from(
        inner
            .height
            .saturating_sub(1 + u16::from(summary.is_some())),
    );
    let start = window(options.len(), app.field, limit);
    let body_size = format!("{} B", draft.body.len());
    let rows = options
        .iter()
        .enumerate()
        .skip(start)
        .take(limit)
        .map(|(index, &field)| {
            let value = match field {
                Field::Media => op
                    .bodies
                    .get(draft.media)
                    .map_or("Raw / custom header", |body| body.media_type.as_str()),
                Field::Auth => draft
                    .auth_option(op)
                    .map_or("None / manual headers", |option| &option.label),
                Field::Secret => {
                    if draft.secret.is_empty() {
                        ""
                    } else {
                        "••••••••"
                    }
                }
                Field::Body if !draft.body.is_empty() => &body_size,
                Field::Body if draft.body_starter(op).is_some() => "Schema starter",
                Field::Headers | Field::Query if !draft.value(field).is_empty() => "[set]",
                _ => draft.value(field),
            };
            let chosen = index == app.field;
            let marker = if chosen { "› " } else { "  " };
            focused(
                Line::from(vec![
                    Span::raw(marker),
                    Span::styled(
                        format!("{:<22} ", safe(&field_label(app, field))),
                        Style::new().fg(MUTED),
                    ),
                    if value.is_empty() {
                        Span::styled("—", Style::new().fg(BORDER))
                    } else {
                        Span::raw(safe(value))
                    },
                ]),
                chosen,
                active,
            )
        });
    frame.render_widget(
        Paragraph::new(Text::from_iter(
            std::iter::once(heading).chain(summary).chain(rows),
        )),
        inner,
    );
}

fn exchange<'a>(app: &'a App, screen: &Screen) -> Option<&'a Result<Response, String>> {
    app.recent
        .iter()
        .filter(|item| item.operation == app.selected)
        .nth(screen.history)
        .map(|item| &item.result)
}

pub(super) fn response_text<'a>(app: &'a App, screen: &Screen) -> Cow<'a, str> {
    match exchange(app, screen) {
        Some(Ok(response)) => match screen.tab {
            ResponseTab::Body => Cow::Borrowed(response.pretty.as_deref().unwrap_or(&response.raw)),
            ResponseTab::Raw => Cow::Borrowed(&response.raw),
            ResponseTab::Headers => Cow::Owned(
                response
                    .headers
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        },
        Some(Err(error)) => Cow::Borrowed(error),
        None if app.pending == Some(app.selected) => Cow::Borrowed("Waiting for the server…"),
        None => Cow::Borrowed("Send a request to see its response."),
    }
}

fn response(frame: &mut Frame<'_>, rect: Rect, app: &App, screen: &Screen) {
    let active = app.pane == Pane::Response;
    let tab = match screen.tab {
        ResponseTab::Body => "Body",
        ResponseTab::Headers => "Headers",
        ResponseTab::Raw => "Raw",
    };
    let block = panel(format!(" Response · {tab} "), active);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if app.matches().is_empty() {
        return;
    }
    let total = app
        .recent
        .iter()
        .filter(|item| item.operation == app.selected)
        .count();
    let summary = match exchange(app, screen) {
        Some(Ok(result)) => {
            let color = if result.status < 300 {
                GOOD
            } else if result.status < 400 {
                WARN
            } else {
                BAD
            };
            let contract = if documented(app.operation(), result.status) {
                "Documented"
            } else {
                "Undocumented status"
            };
            let size = if result.truncated {
                format!("{}+ B · preview", result.bytes)
            } else {
                format!("{} B", result.bytes)
            };
            Line::from(vec![
                Span::styled(result.status.to_string(), Style::new().fg(color).bold()),
                Span::styled(
                    format!(
                        "  {} ms  ·  {size}  ·  {contract}  ·  {}/{}",
                        result.elapsed_ms,
                        screen.history + 1,
                        total
                    ),
                    Style::new().fg(MUTED),
                ),
            ])
        }
        Some(Err(_)) => Line::styled(
            format!("Request failed  ·  {}/{}", screen.history + 1, total),
            Style::new().fg(BAD).bold(),
        ),
        None => Line::default(),
    };
    let text = response_text(app, screen);
    let empty = text.is_empty().then_some(match screen.tab {
        ResponseTab::Headers => "(No headers)",
        _ => "(Empty body)",
    });
    let lines = empty
        .into_iter()
        .chain(text.lines())
        .skip(screen.row)
        .take(usize::from(inner.height.saturating_sub(1)))
        .map(|line| {
            let start = line
                .char_indices()
                .nth(screen.column)
                .map_or(line.len(), |(index, _)| index);
            Line::raw(safe(&line[start..]))
        });
    frame.render_widget(
        Paragraph::new(Text::from_iter(std::iter::once(summary).chain(lines))),
        inner,
    );
}

fn footer(app: &App) -> Line<'static> {
    if let Some(message) = &app.message {
        return Line::styled(format!("  {}", safe(message)), Style::new().fg(BAD));
    }
    let common = if app.pending.is_some() {
        "  Sending…"
    } else if app.can_send() {
        "  Ctrl+S Send"
    } else {
        ""
    };
    let context = match app.pane {
        Pane::Endpoints => "  / Search  Enter Toggle/Open",
        Pane::Request => "  ↑↓ Fields  Enter Edit  ←→ Options",
        Pane::Response => "  ↑↓/←→ Scroll  Enter View  [ ] History",
    };
    let warnings = if app.api.warnings.is_empty() {
        ""
    } else {
        "  ! Import warnings"
    };
    Line::from(vec![
        Span::styled(common, Style::new().fg(ACCENT).bold()),
        Span::styled(
            format!("  v Variables  Tab Focus{context}  ? Help{warnings}"),
            Style::new().fg(MUTED),
        ),
    ])
}

fn variables(frame: &mut Frame<'_>, area: Rect, app: &App, screen: &Screen) {
    let [list, actions] = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(area);
    let entries = app.variables.entries();
    let total = entries.len();
    if total == 0 {
        frame.render_widget(
            Paragraph::new("Press a to add your first variable.").fg(MUTED),
            list,
        );
    } else {
        let visible = usize::from(list.height.saturating_sub(1));
        let start = window(total, screen.variable_row, visible);
        let rows = entries
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(index, (key, value))| {
                let marker = if index == screen.variable_row {
                    "› "
                } else {
                    "  "
                };
                let preview = match value.split_once('\n') {
                    Some((first, _)) => format!("{} ↵", safe(first)),
                    None if value.is_empty() => "—".to_owned(),
                    None => safe(value),
                };
                let row = Row::new([format!("{marker}{key}"), preview]);
                if index == screen.variable_row {
                    return row.style(Style::new().bg(SELECTED).bold());
                }
                row
            });
        let table = Table::new(rows, [Constraint::Percentage(38), Constraint::Fill(1)])
            .header(Row::new(["  Key", "Value"]).style(Style::new().fg(ACCENT).bold()));
        frame.render_widget(table, list);
    }
    let shortcuts = if actions.width >= 44 {
        "a Add  ·  Enter Edit  ·  r Rename  ·  Del Remove"
    } else {
        "a Add ↵ Edit r Key Del"
    };
    let actions_text = Text::from(vec![
        Line::styled(shortcuts, Style::new().fg(MUTED)),
        Line::from(vec![
            Span::styled("Ctrl+S Save  Esc Close", Style::new().fg(ACCENT)),
            Span::raw(
                screen
                    .variable_status
                    .map_or_else(String::new, |status| format!("  ·  {status}")),
            ),
        ]),
    ]);
    frame.render_widget(Paragraph::new(actions_text), actions);
}

fn modal(frame: &mut Frame<'_>, area: Rect, app: &App, screen: &Screen) {
    let Some(overlay) = &screen.overlay else {
        return;
    };
    let width = area.width.saturating_sub(4).min(78);
    let height = match overlay {
        Overlay::Editor(edit, _) if !multiline(edit) => 5,
        Overlay::Editor(..) | Overlay::Variables => 19,
        Overlay::Help => 15,
        Overlay::Warnings => 13,
    }
    .min(area.height.saturating_sub(2));
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let title = match overlay {
        Overlay::Editor(Edit::Search, _) => " Search endpoints ".to_owned(),
        Overlay::Variables => " Variables ".to_owned(),
        Overlay::Editor(Edit::VariableName(None), _) => " Add variable · Key ".to_owned(),
        Overlay::Editor(Edit::VariableName(Some(_)), _) => " Rename variable · Key ".to_owned(),
        Overlay::Editor(Edit::VariableValue(name), _) => format!(" Value · {} ", safe(name)),
        Overlay::Editor(Edit::Request(Field::Body), _)
            if app.draft().body_starter(app.operation()).is_some() =>
        {
            " Body · schema starter ".to_owned()
        }
        Overlay::Editor(Edit::Request(field), _) => {
            format!(" {} ", safe(&field_label(app, *field)))
        }
        Overlay::Help => " Keyboard shortcuts ".to_owned(),
        Overlay::Warnings => format!(" Import warnings · {} ", app.api.warnings.len()),
    };
    let block = panel(title, true).borders(Borders::ALL);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    match overlay {
        Overlay::Editor(edit, editor) => {
            let [content, hint] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);
            frame.render_widget(editor.as_ref(), content);
            let help = match edit {
                Edit::VariableName(None) => "Enter Next  ·  Esc Cancel",
                Edit::VariableName(Some(_)) | Edit::VariableValue(_) => {
                    "Enter Apply  ·  Esc Cancel"
                }
                Edit::Request(Field::Headers | Field::Query) => {
                    "One name: value per line  ·  Ctrl+S Save  ·  Esc Cancel"
                }
                Edit::Request(Field::Body) => "Ctrl+S Save  ·  Esc Cancel",
                _ => "Enter Save  ·  Esc Cancel",
            };
            frame.render_widget(Paragraph::new(help).fg(MUTED), hint);
        }
        Overlay::Variables => variables(frame, inner, app, screen),
        Overlay::Help => {
            let help = "Tab / Shift+Tab   Focus the next or previous pane\n↑↓ / PgUp / PgDn   Move or scroll\n/                 Search methods, paths, summaries and tags\nEsc               Close a dialog or clear the search\nEnter             Toggle folder, open endpoint, edit field or cycle view\n← →               Fold/unfold, cycle options, or scroll response\nCtrl+S            Save editor or send request\n[ / ]             Older or newer response for this endpoint\nv                 Edit shared request variables\n!                 Show import warnings\nq / Ctrl+C        Quit\n* Required by spec  ·  † Complex parameter, see README";
            frame.render_widget(
                Paragraph::new(help).fg(TEXT).wrap(Wrap { trim: false }),
                inner,
            );
        }
        Overlay::Warnings => {
            let text = if app.api.warnings.is_empty() {
                "No import warnings.".to_owned()
            } else {
                app.api
                    .warnings
                    .iter()
                    .enumerate()
                    .skip(screen.warning_row)
                    .take(usize::from(inner.height))
                    .map(|(index, warning)| format!("{}. {}", index + 1, safe(warning)))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            frame.render_widget(
                Paragraph::new(text).fg(WARN).wrap(Wrap { trim: false }),
                inner,
            );
        }
    }
}

pub(super) fn render(frame: &mut Frame<'_>, app: &App, screen: &Screen) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::new().bg(BACKGROUND).fg(TEXT)),
        area,
    );
    if area.width < 30 || area.height < 8 {
        frame.render_widget(Paragraph::new("Resize to at least 30 × 8").fg(WARN), area);
        return;
    }
    let [top, workspace, bottom] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(area);
    let target = if app.matches().is_empty() {
        String::new()
    } else {
        let op = app.operation();
        let draft = app.draft();
        destination(op, &draft, &app.variables).map_or_else(
            |error| format!("  {}", safe(&error)),
            |url| format!("  {url}{}", if draft.has_query(op) { "?…" } else { "" }),
        )
    };
    let header = Text::from(vec![
        Line::from(vec![
            Span::styled("  viscurl", Style::new().fg(ACCENT).bold()),
            Span::styled(
                format!("  /  {}", safe(&app.api.title)),
                Style::new().fg(TEXT).bold(),
            ),
        ]),
        Line::styled(target, Style::new().fg(MUTED)),
    ]);
    frame.render_widget(Paragraph::new(header), top);
    if area.width >= 112 && area.height >= 14 {
        let [navigation, builder, result] = Layout::horizontal([
            Constraint::Percentage(24),
            Constraint::Percentage(38),
            Constraint::Percentage(38),
        ])
        .spacing(1)
        .areas(workspace);
        endpoints(frame, navigation, app);
        request(frame, builder, app);
        response(frame, result, app, screen);
    } else if area.width >= 72 {
        let [navigation, details] =
            Layout::horizontal([Constraint::Percentage(33), Constraint::Fill(1)]).areas(workspace);
        endpoints(frame, navigation, app);
        match app.pane {
            Pane::Response => response(frame, details, app, screen),
            _ => request(frame, details, app),
        }
    } else {
        match app.pane {
            Pane::Endpoints => endpoints(frame, workspace, app),
            Pane::Request => request(frame, workspace, app),
            Pane::Response => response(frame, workspace, app, screen),
        }
    }
    frame.render_widget(Paragraph::new(footer(app)), bottom);
    modal(frame, area, app, screen);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn frame(app: &App, screen: &Screen, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app, screen)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn lays_out_wide_and_narrow_views_and_masks_credentials() {
        let api =
            crate::domain::spec::parse(include_str!("../../../tests/fixtures/pets.yaml")).unwrap();
        let app = App::new(api)
            .cycle(Field::Auth)
            .edit(Field::Secret, "hidden-test-value".into());
        let screen = Screen::default();
        let wide = frame(&app, &screen, 112, 30);
        assert!(
            ["Endpoints", "Request", "Response", "••••••••"]
                .into_iter()
                .all(|label| wide.contains(label))
        );
        assert!(!wide.contains("hidden-test-value"));
        let app = app.focus(Pane::Response);
        let narrow = frame(&app, &screen, 56, 18);
        assert!(narrow.contains("Response · Body"));
        assert!(!narrow.contains("Endpoints"));
    }
}
