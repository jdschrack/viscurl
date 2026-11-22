mod view;

use crate::{
    adapters::{config, http},
    application::{App, Pane, TreeAction},
    domain::{request::Field, spec::Api},
};
use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
};
use ratatui::{
    DefaultTerminal,
    style::{Color, Modifier, Style},
};
use ratatui_textarea::{CursorMove, TextArea};
use std::{io, sync::mpsc, thread, time::Duration};

const MAX_EDIT_BYTES: usize = 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
enum Edit {
    Search,
    VariableName(Option<String>),
    VariableValue(String),
    Request(Field),
}

enum Overlay {
    Editor(Edit, Box<TextArea<'static>>),
    Variables,
    Help,
    Warnings,
}

#[derive(Clone, Copy, Default)]
enum ResponseTab {
    #[default]
    Body,
    Headers,
    Raw,
}

impl ResponseTab {
    fn next(self) -> Self {
        match self {
            Self::Body => Self::Headers,
            Self::Headers => Self::Raw,
            Self::Raw => Self::Body,
        }
    }
}

#[derive(Default)]
struct Screen {
    overlay: Option<Overlay>,
    tab: ResponseTab,
    row: usize,
    column: usize,
    history: usize,
    warning_row: usize,
    variable_row: usize,
    variable_status: Option<&'static str>,
}

impl Screen {
    fn reset_response(&mut self) {
        self.row = 0;
        self.column = 0;
        self.history = 0;
    }
}

fn fields(app: &App) -> Vec<Field> {
    let op = app.operation();
    std::iter::once(Field::Server)
        .chain((0..op.parameters.len()).map(Field::Parameter))
        .chain([
            Field::Query,
            Field::Headers,
            Field::Media,
            Field::Body,
            Field::Auth,
        ])
        .chain((app.draft().auth != 0).then_some(Field::Secret))
        .collect()
}

fn multiline(edit: &Edit) -> bool {
    matches!(
        edit,
        Edit::VariableValue(_) | Edit::Request(Field::Query | Field::Headers | Field::Body)
    )
}

fn editor_text(edit: &Edit, value: &str) -> String {
    if multiline(edit) {
        return value
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .chars()
            .map(|ch| {
                if matches!(ch, '\n' | '\t') {
                    ch
                } else {
                    view::safe_char(ch)
                }
            })
            .collect();
    }
    view::safe(&value.replace(['\n', '\r', '\t'], " "))
}

fn editor(edit: Edit, value: &str) -> Overlay {
    let mut area = TextArea::new(
        editor_text(&edit, value)
            .split('\n')
            .map(str::to_owned)
            .collect(),
    );
    area.move_cursor(CursorMove::End);
    area.set_cursor_line_style(Style::default());
    area.set_max_histories(12);
    area.set_cursor_style(
        Style::default()
            .fg(Color::Black)
            .bg(view::ACCENT)
            .add_modifier(Modifier::BOLD),
    );
    if edit == Edit::Request(Field::Secret) {
        area.set_mask_char('•');
        area.set_max_histories(0);
    }
    Overlay::Editor(edit, Box::new(area))
}

fn edit_event(edit: &Edit, area: &mut TextArea<'_>, input: &Event) {
    let bytes = area
        .lines()
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>();
    match input {
        Event::Key(key)
            if key.kind != KeyEventKind::Release
                && (multiline(edit) || !matches!(key.code, KeyCode::Enter | KeyCode::Tab)) =>
        {
            if (bytes + 4 <= MAX_EDIT_BYTES
                || !matches!(key.code, KeyCode::Char(_) | KeyCode::Enter | KeyCode::Tab))
                && !matches!(key.code, KeyCode::Char(ch) if ch.is_control())
            {
                area.input(*key);
            }
        }
        Event::Paste(text) if text.len() <= MAX_EDIT_BYTES => {
            let text = editor_text(edit, text);
            if bytes.saturating_add(text.len()) <= MAX_EDIT_BYTES {
                area.insert_str(text);
            }
        }
        _ => {}
    }
}

fn control(key: KeyEvent, ch: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(ch)
}

fn request_key(app: App, screen: &mut Screen, key: KeyCode, step: Option<isize>) -> App {
    let options = fields(&app);
    if let Some(step) = step {
        let field = app.field.saturating_add_signed(step).min(options.len() - 1);
        return app.select_field(field);
    }
    let field = options[app.field.min(options.len() - 1)];
    if matches!(key, KeyCode::Left | KeyCode::Right)
        && matches!(field, Field::Server | Field::Auth | Field::Media)
    {
        return app.cycle(field);
    }
    if key == KeyCode::Enter {
        if matches!(field, Field::Auth | Field::Media) {
            return app.cycle(field);
        }
        let draft = app.draft();
        let starter = if field == Field::Body {
            draft.body_starter(app.operation())
        } else {
            None
        };
        screen.overlay = Some(editor(
            Edit::Request(field),
            starter.unwrap_or_else(|| draft.value(field)),
        ));
    }
    app
}

fn response_key(app: &App, screen: &mut Screen, key: KeyCode, step: Option<isize>) {
    if let Some(step) = step {
        let last = view::response_text(app, screen)
            .lines()
            .count()
            .saturating_sub(1);
        screen.row = screen.row.saturating_add_signed(step).min(last);
        return;
    }
    match key {
        KeyCode::Left => screen.column = screen.column.saturating_sub(12),
        KeyCode::Right => {
            let longest = view::response_text(app, screen)
                .lines()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or_default();
            screen.column = (screen.column + 12).min(longest.saturating_sub(1));
        }
        KeyCode::Enter => {
            screen.tab = screen.tab.next();
            screen.row = 0;
            screen.column = 0;
        }
        KeyCode::Char('[' | ']') => {
            let latest = app
                .recent
                .iter()
                .filter(|item| item.operation == app.selected)
                .count()
                .saturating_sub(1);
            screen.history = if key == KeyCode::Char('[') {
                (screen.history + 1).min(latest)
            } else {
                screen.history.saturating_sub(1)
            };
            screen.row = 0;
            screen.column = 0;
        }
        _ => {}
    }
}

enum Action {
    Continue(App),
    Send(App),
    SaveVariables(App),
    Quit,
}

fn navigation_step(code: KeyCode) -> Option<isize> {
    match code {
        KeyCode::Up => Some(-1),
        KeyCode::Down => Some(1),
        KeyCode::PageUp => Some(-10),
        KeyCode::PageDown => Some(10),
        _ => None,
    }
}

fn variable_row_key(app: App, screen: &mut Screen, key: KeyEvent) -> Action {
    if control(key, 's') {
        return Action::SaveVariables(app);
    }
    if let Some(step) = navigation_step(key.code) {
        screen.variable_row = screen
            .variable_row
            .saturating_add_signed(step)
            .min(app.variables.entries().len().saturating_sub(1));
        return Action::Continue(app);
    }
    let entry = app.variables.entries().nth(screen.variable_row);
    let app = match key.code {
        KeyCode::Esc | KeyCode::Char('v') => {
            screen.overlay = None;
            app
        }
        KeyCode::Char('a') => {
            screen.overlay = Some(editor(Edit::VariableName(None), ""));
            app
        }
        KeyCode::Enter => {
            if let Some((name, value)) = entry {
                screen.overlay = Some(editor(Edit::VariableValue(name.to_owned()), value));
            }
            app
        }
        KeyCode::Char('r') => {
            if let Some((name, _)) = entry {
                screen.overlay = Some(editor(Edit::VariableName(Some(name.to_owned())), name));
            }
            app
        }
        KeyCode::Delete => {
            if let Some((name, _)) = entry {
                let variables = app.variables.remove(name);
                screen.variable_row = screen
                    .variable_row
                    .min(variables.entries().len().saturating_sub(1));
                screen.variable_status = Some("Session changes");
                app.with_variables(variables)
            } else {
                app
            }
        }
        _ => app,
    };
    Action::Continue(app)
}

fn apply_variable(
    app: App,
    screen: &mut Screen,
    previous: Option<&str>,
    name: String,
    value: String,
) -> Action {
    if previous == Some(name.as_str()) && app.variables.value(&name) == Some(value.as_str()) {
        screen.overlay = Some(Overlay::Variables);
        return Action::Continue(app.clear_message());
    }
    match app.variables.set(previous, name.clone(), value) {
        Ok(variables) => {
            screen.variable_row = variables
                .entries()
                .position(|(key, _)| key == name)
                .unwrap_or_default();
            screen.variable_status = Some("Session changes");
            screen.overlay = Some(Overlay::Variables);
            Action::Continue(app.with_variables(variables))
        }
        Err(error) => Action::Continue(app.with_message(error)),
    }
}

fn apply_edit(app: App, screen: &mut Screen, edit: Edit, value: String) -> Action {
    if value.len() > MAX_EDIT_BYTES {
        return Action::Continue(app.with_message("Edited text exceeds 1 MiB".into()));
    }
    match edit {
        Edit::Search => {
            screen.overlay = None;
            screen.reset_response();
            Action::Continue(app.filter(value).focus(Pane::Endpoints))
        }
        Edit::Request(field) => {
            screen.overlay = None;
            Action::Continue(app.edit(field, value))
        }
        Edit::VariableName(previous) => {
            let name = value.trim().to_owned();
            if let Some(previous) = previous {
                let value = app
                    .variables
                    .value(&previous)
                    .unwrap_or_default()
                    .to_owned();
                return apply_variable(app, screen, Some(&previous), name, value);
            }
            if let Err(error) = app.variables.check_name(&name, None) {
                return Action::Continue(app.with_message(error));
            }
            screen.overlay = Some(editor(Edit::VariableValue(name), ""));
            Action::Continue(app.clear_message())
        }
        Edit::VariableValue(ref name) => {
            let value = match app.variables.value(name) {
                Some(original) if editor_text(&edit, original) == value => original.to_owned(),
                _ => value,
            };
            apply_variable(app, screen, Some(name), name.clone(), value)
        }
    }
}

fn handle(app: App, screen: &mut Screen, input: Event) -> Action {
    let key = match input {
        Event::Key(key) if key.kind != KeyEventKind::Release => Some(key),
        _ => None,
    };
    if key.is_some_and(|key| control(key, 'c')) {
        return Action::Quit;
    }
    if let Some(overlay) = screen.overlay.as_mut() {
        match overlay {
            Overlay::Editor(edit, area) => {
                if key.is_some_and(|key| {
                    control(key, 's')
                        || (key.code == KeyCode::Enter
                            && (!multiline(edit)
                                || (key.modifiers.is_empty()
                                    && matches!(edit, Edit::VariableValue(_)))))
                }) {
                    let edit = edit.clone();
                    let value = area.lines().join("\n");
                    return apply_edit(app, screen, edit, value);
                }
                if key.is_some_and(|key| key.code == KeyCode::Esc) {
                    let variable = matches!(edit, Edit::VariableName(_) | Edit::VariableValue(_));
                    screen.overlay = variable.then_some(Overlay::Variables);
                    return Action::Continue(if variable { app.clear_message() } else { app });
                }
                edit_event(edit, area, &input);
            }
            Overlay::Variables => {
                if let Some(key) = key {
                    return variable_row_key(app, screen, key);
                }
            }
            Overlay::Help | Overlay::Warnings => match key.map(|key| key.code) {
                Some(KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?' | '!')) => {
                    screen.overlay = None
                }
                Some(KeyCode::Down) => {
                    screen.warning_row =
                        (screen.warning_row + 1).min(app.api.warnings.len().saturating_sub(1))
                }
                Some(KeyCode::Up) => screen.warning_row = screen.warning_row.saturating_sub(1),
                _ => {}
            },
        }
        return Action::Continue(app);
    }
    let Some(key) = key else {
        return Action::Continue(app);
    };
    if control(key, 's') && app.can_send() {
        return Action::Send(app);
    }
    let step = navigation_step(key.code);
    Action::Continue(match key.code {
        KeyCode::Char('q') => return Action::Quit,
        KeyCode::Tab | KeyCode::BackTab => {
            let pane = app.pane.step(key.code == KeyCode::BackTab);
            app.focus(pane)
        }
        KeyCode::Char('/') => {
            screen.overlay = Some(editor(Edit::Search, &app.search));
            app
        }
        KeyCode::Char('v') => {
            screen.overlay = Some(Overlay::Variables);
            app
        }
        KeyCode::Char('?') => {
            screen.overlay = Some(Overlay::Help);
            app
        }
        KeyCode::Char('!') => {
            screen.warning_row = 0;
            screen.overlay = Some(Overlay::Warnings);
            app
        }
        KeyCode::Esc if !app.search.is_empty() => {
            screen.reset_response();
            app.filter(String::new())
        }
        _ if app.matches().is_empty() => app,
        _ => match app.pane {
            Pane::Endpoints if step.is_some() => {
                screen.reset_response();
                app.move_operation(step.unwrap_or_default())
            }
            Pane::Endpoints => match key.code {
                KeyCode::Enter => app.tree_action(TreeAction::Open),
                KeyCode::Right => app.tree_action(TreeAction::Expand),
                KeyCode::Left => app.tree_action(TreeAction::Collapse),
                _ => app,
            },
            Pane::Request => request_key(app, screen, key.code, step),
            Pane::Response => {
                response_key(&app, screen, key.code, step);
                app
            }
        },
    })
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    api: Api,
    client: reqwest::blocking::Client,
) -> io::Result<()> {
    let (sender, receiver) = mpsc::channel();
    let configuration = config::path()
        .ok_or("No absolute configuration directory is set".to_owned())
        .and_then(|path| config::load(&path).map(|variables| (path, variables)));
    let (mut app, config_path) = match configuration {
        Ok((path, variables)) => (App::new(api).with_variables(variables), Ok(path)),
        Err(error) => (
            App::new(api).with_message(format!("Cannot load variables: {error}")),
            Err(error),
        ),
    };
    let mut screen = Screen::default();
    let mut dirty = true;
    loop {
        if let Ok(result) = receiver.try_recv() {
            app = app.finish(result);
            screen.reset_response();
            dirty = true;
        }
        if dirty {
            terminal.draw(|frame| view::render(frame, &app, &screen))?;
            dirty = false;
        }
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        dirty = true;
        app = match handle(app, &mut screen, event::read()?) {
            Action::Continue(app) => app,
            Action::Quit => return Ok(()),
            Action::SaveVariables(app) => {
                match config_path.as_ref().map_err(Clone::clone).and_then(|path| {
                    config::save(path, &app.variables).map_err(|error| error.to_string())
                }) {
                    Ok(()) => {
                        screen.variable_status = Some("Saved to config");
                        app.clear_message()
                    }
                    Err(error) => app.with_message(format!("Cannot save variables: {error}")),
                }
            }
            Action::Send(app) => {
                let (app, request) = app.start();
                if let Some(request) = request {
                    screen.reset_response();
                    let (client, sender) = (client.clone(), sender.clone());
                    if thread::Builder::new()
                        .spawn(move || {
                            let _ = sender.send(http::send(&client, request));
                        })
                        .is_err()
                    {
                        app.finish(Err("Could not start the HTTP request".into()))
                    } else {
                        app
                    }
                } else {
                    app
                }
            }
        };
    }
}

pub fn run(api: Api) -> io::Result<()> {
    let client = http::client().map_err(io::Error::other)?;
    let result = ratatui::try_init().and_then(|mut terminal| {
        execute!(io::stdout(), EnableBracketedPaste)?;
        event_loop(&mut terminal, api, client)
    });
    let paste = execute!(io::stdout(), DisableBracketedPaste);
    let restore = ratatui::try_restore();
    result.and(paste).and(restore)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_schema_starter_without_adding_it_to_the_request_until_saved() {
        let api = crate::domain::spec::parse(
            r#"
openapi: 3.1.0
servers: [{url: 'http://localhost:8080'}]
paths:
  /login:
    post:
      requestBody:
        content:
          application/json:
            schema: {$ref: '#/components/schemas/Login'}
components:
  schemas:
    Login:
      type: object
      required: [email, password]
      properties:
        email: {type: string}
        password: {type: string}
        optional: {type: boolean}
"#,
        )
        .unwrap();
        let app = App::new(api).focus(Pane::Request);
        let field = fields(&app)
            .iter()
            .position(|&field| field == Field::Body)
            .unwrap();
        let app = app.select_field(field);
        let mut screen = Screen::default();
        let app = request_key(app, &mut screen, KeyCode::Enter, None);
        let Some(Overlay::Editor(_, editor)) = &screen.overlay else {
            panic!("body editor should open");
        };
        let sample = editor.lines().join("\n");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&sample).unwrap(),
            serde_json::json!({"email": "", "password": ""})
        );
        assert!(app.draft().body.is_empty());

        let save = Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        let Action::Continue(app) = handle(app, &mut screen, save) else {
            panic!("saving should update the body");
        };
        let (_, request) = app.start();
        assert_eq!(request.unwrap().body, sample);
    }

    #[test]
    fn saves_multiline_json_with_real_newlines_and_preserves_string_escapes() {
        let api =
            crate::domain::spec::parse(include_str!("../../../tests/fixtures/pets.yaml")).unwrap();
        let mut screen = Screen {
            overlay: Some(editor(Edit::Request(Field::Body), "")),
            ..Screen::default()
        };
        let key = |code, modifiers| Event::Key(KeyEvent::new(code, modifiers));
        let events = [
            Event::Paste("{\r\n".into()),
            Event::Paste(r#"  "name": "Fern\nOak""#.into()),
            key(KeyCode::Enter, KeyModifiers::NONE),
            Event::Paste("}".into()),
            key(KeyCode::Char('s'), KeyModifiers::CONTROL),
        ];
        let app = events.into_iter().fold(App::new(api), |app, event| {
            match handle(app, &mut screen, event) {
                Action::Continue(app) => app,
                _ => panic!("saving the editor should update the request draft"),
            }
        });
        let (_, request) = app.start();
        let request = request.unwrap();
        assert_eq!(request.body, "{\n  \"name\": \"Fern\\nOak\"\n}");
        assert_eq!(request.headers["content-type"], "application/json");
        let json: serde_json::Value = serde_json::from_str(&request.body).unwrap();
        assert_eq!(json["name"], "Fern\nOak");
    }

    #[test]
    fn variables_editor_adds_edits_renames_and_removes_until_explicit_save() {
        let api =
            crate::domain::spec::parse(include_str!("../../../tests/fixtures/pets.yaml")).unwrap();
        let mut screen = Screen::default();
        let mut input = |app, event| match handle(app, &mut screen, event) {
            Action::Continue(app) => app,
            _ => panic!("edits should only change the session"),
        };
        let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        let text = |text: &str| Event::Paste(text.to_owned());
        let app = [
            key(KeyCode::Char('v')),
            key(KeyCode::Char('a')),
            text("base_url"),
            key(KeyCode::Enter),
            text("http://localhost:3000"),
            key(KeyCode::Enter),
            key(KeyCode::Char('a')),
            text("mode"),
            key(KeyCode::Enter),
            text("staging"),
            key(KeyCode::Enter),
            key(KeyCode::Char('r')),
            text("_name"),
            key(KeyCode::Enter),
            key(KeyCode::Enter),
            text("-local"),
            key(KeyCode::Enter),
            key(KeyCode::Up),
            key(KeyCode::Delete),
            key(KeyCode::Esc),
            key(KeyCode::Char('v')),
        ]
        .into_iter()
        .fold(App::new(api), &mut input);
        assert_eq!(
            app.variables.entries().collect::<Vec<_>>(),
            [("mode_name", "staging-local")]
        );
        assert_eq!(screen.variable_status, Some("Session changes"));
        let save = Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(matches!(
            handle(app, &mut screen, save),
            Action::SaveVariables(_)
        ));
    }

    #[test]
    fn multiline_editor_keeps_tabs_and_normalizes_pasted_line_endings() {
        assert_eq!(
            editor_text(&Edit::Request(Field::Body), "a\tb\r\nc\u{1b}"),
            "a\tb\nc�"
        );
        assert_eq!(editor_text(&Edit::Search, "line\nbreak"), "line break");
    }
}
