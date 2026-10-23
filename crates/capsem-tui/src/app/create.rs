use super::*;
use capsem_sdk::models::{ImageInfo, ImageListResponse};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImageCatalog {
    Loading,
    Loaded(Vec<ImageInfo>),
    Failed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageCatalogRequest {
    pub generation: u64,
    pub refresh: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CreateField {
    Name,
    Image,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateDraft {
    pub name: String,
    pub field: CreateField,
    pub catalog: ImageCatalog,
    pub image_name: Option<String>,
    pub caching: bool,
    generation: u64,
}

impl CreateDraft {
    pub fn selected_image(&self) -> Option<&ImageInfo> {
        let ImageCatalog::Loaded(images) = &self.catalog else {
            return None;
        };
        images
            .iter()
            .find(|image| Some(&image.name) == self.image_name.as_ref())
    }
}

impl App {
    pub(super) fn open_create(&mut self) {
        if self.pending_create.is_some() {
            self.set_control_message("a session creation is already pending");
            return;
        }
        self.pending_action = None;
        self.fork_draft = None;
        self.catalog_generation = self
            .catalog_generation
            .checked_add(1)
            .expect("catalog generation overflow");
        self.catalog_request = Some(ImageCatalogRequest {
            generation: self.catalog_generation,
            refresh: true,
        });
        self.create_draft = Some(CreateDraft {
            name: next_session_name(&self.state),
            field: CreateField::Name,
            catalog: ImageCatalog::Loading,
            image_name: None,
            caching: false,
            generation: self.catalog_generation,
        });
        self.overlay = AppOverlay::Create;
    }

    pub fn take_catalog_request(&mut self) -> Option<ImageCatalogRequest> {
        self.catalog_request.take().filter(|request| {
            self.create_draft
                .as_ref()
                .is_some_and(|draft| draft.generation == request.generation)
        })
    }

    pub fn retry_catalog_request(&mut self, request: ImageCatalogRequest) {
        if self
            .create_draft
            .as_ref()
            .is_some_and(|draft| draft.generation == request.generation)
        {
            self.catalog_request = Some(request);
        }
    }

    pub fn apply_image_catalog(&mut self, request: ImageCatalogRequest, result: Result<ImageListResponse, String>) {
        if let Some(draft) = self
            .create_draft
            .as_mut()
            .or(self.pending_create.as_mut())
            .filter(|draft| draft.generation == request.generation)
        {
            draft.catalog = match result {
                Ok(response) => ImageCatalog::Loaded(
                    response
                        .images
                        .into_iter()
                        .filter(|image| image.surface == Some(capsem_sdk::models::ImageSurface::Terminal))
                        .collect(),
                ),
                Err(error) => ImageCatalog::Failed(error),
            };
        }
    }

    pub fn refresh_image_catalog(&mut self) {
        let Some(draft) = &mut self.create_draft else { return };
        self.catalog_generation = self
            .catalog_generation
            .checked_add(1)
            .expect("catalog generation overflow");
        draft.generation = self.catalog_generation;
        draft.catalog = ImageCatalog::Loading;
        self.catalog_request = Some(ImageCatalogRequest {
            generation: self.catalog_generation,
            refresh: true,
        });
    }

    pub(super) fn poll_pending_image_catalog(&mut self) {
        if self.overlay != AppOverlay::Create
            || self.state.service.status != ServiceStatus::Online
            || self.catalog_request.is_some()
        {
            return;
        }
        let Some(draft) = &self.create_draft else { return };
        let pending = matches!(&draft.catalog, ImageCatalog::Loaded(images) if images.iter().any(|image| {
            image.image.is_some() && (image.cached == capsem_sdk::models::ImageCacheState::Unknown || image.progress.is_some())
        }));
        if draft.caching || pending {
            self.catalog_request = Some(ImageCatalogRequest {
                generation: draft.generation,
                refresh: false,
            });
        }
    }

    pub fn complete_image_cache(&mut self, success: bool) {
        if let Some(draft) = &mut self.create_draft {
            draft.caching = false;
            if success {
                self.refresh_image_catalog();
            }
        }
    }

    pub fn complete_create(&mut self, success: bool) {
        let Some(draft) = self.pending_create.take() else {
            return;
        };
        if !success && self.overlay == AppOverlay::None {
            self.create_draft = Some(draft);
            self.overlay = AppOverlay::Create;
        }
    }

    pub(super) fn handle_create_key(&mut self, key: KeyEvent) -> AppAction {
        let Some(draft) = &mut self.create_draft else {
            return AppAction::Consumed;
        };
        if draft.caching && key.code != KeyCode::Esc {
            return AppAction::Consumed;
        }
        match key.code {
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.refresh_image_catalog();
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let Some(image) = draft.selected_image().filter(|image| image.image.is_some()) else {
                    self.set_control_message("choose a compatible image to cache");
                    return AppAction::Consumed;
                };
                let action = ControlAction::CacheImage {
                    image: image.image.clone().expect("compatible image"),
                    name: image.name.clone(),
                };
                draft.caching = true;
                return AppAction::Invoke(action);
            }
            KeyCode::Esc => {
                self.create_draft = None;
                self.catalog_request = None;
                self.overlay = AppOverlay::None;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                draft.field = match draft.field {
                    CreateField::Name => CreateField::Image,
                    CreateField::Image => CreateField::Name,
                };
            }
            KeyCode::Up | KeyCode::Down if draft.field == CreateField::Image => {
                if let ImageCatalog::Loaded(images) = &draft.catalog {
                    let count = images.len();
                    if count == 0 {
                        return AppAction::Consumed;
                    }
                    let selection = draft
                        .image_name
                        .as_ref()
                        .and_then(|name| images.iter().position(|image| &image.name == name))
                        .unwrap_or(count - 1);
                    let selection = if key.code == KeyCode::Down {
                        (selection + 1) % count
                    } else {
                        (selection + count - 1) % count
                    };
                    draft.image_name = Some(images[selection].name.clone());
                }
            }
            KeyCode::Enter => {
                let name = draft.name.trim().to_string();
                if name.is_empty() {
                    return AppAction::Consumed;
                }
                let image = {
                    let Some(image) = draft.selected_image().filter(|image| image.image.is_some()) else {
                        self.set_control_message("choose a compatible terminal image; Tab then Up/Down");
                        return AppAction::Consumed;
                    };
                    // Use the advertised immutable pin. The service rechecks admission.
                    image.image.clone()
                };
                let name = (name != next_session_name(&self.state)).then_some(name);
                self.pending_create = self.create_draft.take();
                self.catalog_request = None;
                self.overlay = AppOverlay::None;
                return AppAction::Invoke(ControlAction::CreateSession { name, image });
            }
            KeyCode::Backspace if draft.field == CreateField::Name => {
                draft.name.pop();
            }
            KeyCode::Char(ch)
                if draft.field == CreateField::Name
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
            {
                draft.name.push(ch);
            }
            _ => {}
        }
        AppAction::Consumed
    }
}
