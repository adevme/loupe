use iced::widget::{center, image};
use iced::{keyboard, window, Element, Subscription, Task, Theme};

use crate::settings::Settings;
use crate::theme::{self, Palette};
use crate::{App, Message};

const ICON_SIZE: f32 = 160.0;

pub enum Loupe {
    Starting { main: window::Id, palette: Palette, scale: f64, waiting: Option<Box<Waiting>> },
    Ready(Box<App>),
}

pub struct Waiting {
    main: window::Id,
    loaded: theme::Loaded,
    settings: Settings,
    shift_at_start: bool,
}

fn splash_icon() -> image::Handle {
    static ICON: std::sync::OnceLock<image::Handle> = std::sync::OnceLock::new();
    ICON.get_or_init(|| image::Handle::from_bytes(include_bytes!("../assets/icon.png").as_slice())).clone()
}

impl Loupe {
    pub fn starting(main: window::Id, loaded: theme::Loaded, settings: Settings, shift_at_start: bool) -> (Self, Task<Message>) {
        let palette = loaded.palette;
        let scale = settings.scale;
        let waiting = Some(Box::new(Waiting { main, loaded, settings, shift_at_start }));
        (Loupe::Starting { main, palette, scale, waiting }, Task::none())
    }

    pub fn title(&self, window: window::Id) -> String {
        match self {
            Loupe::Ready(app) => app.title_of(window),
            Loupe::Starting { .. } => "Loupe".into(),
        }
    }

    pub fn theme(&self, _window: window::Id) -> Theme {
        match self {
            Loupe::Ready(app) => app.palette.iced(),
            Loupe::Starting { palette, .. } => palette.iced(),
        }
    }

    pub fn scale(&self, _window: window::Id) -> f64 {
        match self {
            Loupe::Ready(app) => app.scale,
            Loupe::Starting { scale, .. } => *scale,
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let waiting = match self {
            Loupe::Ready(app) => return app.update(message),
            Loupe::Starting { waiting, .. } => waiting,
        };
        match message {
            Message::ModifiersChanged(modifiers) if modifiers.shift() => {
                if let Some(waiting) = waiting.as_mut() {
                    waiting.shift_at_start = true;
                }
                Task::none()
            }
            Message::FirstFrame => {
                let Some(waiting) = waiting.take() else { return Task::none() };
                let Waiting { main, loaded, settings, shift_at_start } = *waiting;
                let (app, task) = App::new(main, loaded, settings, shift_at_start);
                *self = Loupe::Ready(Box::new(app));
                task
            }
            _ => Task::none(),
        }
    }

    pub fn view(&self, window: window::Id) -> Element<'_, Message> {
        match self {
            Loupe::Ready(app) => app.view_of(window),
            Loupe::Starting { main, palette, .. } => {
                let palette = *palette;
                let _ = main;
                center(image(splash_icon()).width(ICON_SIZE).height(ICON_SIZE))
                    .style(move |_| iced::widget::container::Style { background: Some(palette.background.into()), ..Default::default() })
                    .into()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        match self {
            Loupe::Ready(app) => app.subscription(),
            Loupe::Starting { .. } => Subscription::batch([
                window::frames().map(|_| Message::FirstFrame),
                iced::event::listen_with(|event, _status, _window| match event {
                    iced::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => Some(Message::ModifiersChanged(modifiers)),
                    _ => None,
                }),
            ]),
        }
    }
}
