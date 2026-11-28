//! The eframe application: owns the state and the connection worker and runs the UI.

use std::time::{Duration, Instant};

use eframe::egui;
use mqtt_viewer_core::connection::Connection;

use crate::state::AppState;
use crate::ui;

/// Longest time spent applying broker events per frame, so a flood cannot freeze the UI.
const EVENT_BUDGET: Duration = Duration::from_millis(8);

/// The running application.
#[derive(Debug)]
pub struct App {
    state: AppState,
    connection: Connection,
}

impl App {
    /// Creates the app for a new window, loading the saved configuration.
    ///
    /// # Errors
    /// Fails when the connection worker cannot be started.
    pub fn new(creation: &eframe::CreationContext<'_>) -> std::io::Result<Self> {
        Self::with_state(&creation.egui_ctx, AppState::load())
    }

    /// Creates the app around an existing state.
    ///
    /// # Errors
    /// Fails when the connection worker cannot be started.
    pub fn with_state(ctx: &egui::Context, mut state: AppState) -> std::io::Result<Self> {
        let repaint = ctx.clone();
        let connection = Connection::spawn(move || repaint.request_repaint())?;
        state.start();
        Ok(Self { state, connection })
    }

    /// The application state.
    #[must_use]
    pub const fn state(&self) -> &AppState {
        &self.state
    }

    /// The application state, for driving it from tests.
    pub const fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    /// Applies pending broker events and sends queued commands. Returns whether events are still waiting.
    pub fn pump(&mut self) -> bool {
        let started = Instant::now();
        let mut more = false;
        while let Some(event) = self.connection.try_event() {
            self.state.handle_event(event);
            if started.elapsed() > EVENT_BUDGET {
                more = true;
                break;
            }
        }
        self.state.tick(Instant::now());
        for command in self.state.take_commands() {
            self.connection.send(command);
        }
        more
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.pump() {
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::root(ui, &mut self.state);
        // Commands raised by this frame's input go out right away instead of waiting for the next repaint.
        for command in self.state.take_commands() {
            self.connection.send(command);
        }
    }
}
