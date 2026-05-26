use std::{ffi::c_void, thread, time::Duration};

use synapse_core::Rect;
use windows_capture::{
    capture::{Context, GraphicsCaptureApiHandler},
    dxgi_duplication_api::{DxgiDuplicationApi, DxgiDuplicationFormat, Error as DxgiError},
    frame::{DirtyRegion, Frame},
    graphics_capture_api::InternalCaptureControl,
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
    window::Window,
};

use crate::{
    CaptureConfig, CaptureError, CaptureTarget, CapturedFrame, DxgiFormat, SendablePtr,
    controller::{CaptureThreadContext, push_frame},
};

use super::{common::capture_unsupported, target::validate_hwnd};

pub fn run_graphics_capture(
    config: CaptureConfig,
    ctx: CaptureThreadContext,
) -> Result<(), CaptureError> {
    match config.target.clone() {
        CaptureTarget::Primary => {
            let monitor = Monitor::primary().map_err(capture_unsupported)?;
            start_graphics_capture_with_item(monitor, config, ctx)
        }
        CaptureTarget::Monitor { monitor_index } => {
            let monitor =
                Monitor::from_index(usize::try_from(monitor_index.saturating_add(1)).map_err(
                    |err| CaptureError::TargetInvalid {
                        detail: err.to_string(),
                    },
                )?)
                .map_err(|err| CaptureError::TargetInvalid {
                    detail: err.to_string(),
                })?;
            start_graphics_capture_with_item(monitor, config, ctx)
        }
        CaptureTarget::Window { hwnd } => {
            validate_hwnd(hwnd)?;
            let window = Window::from_raw_hwnd(hwnd as *mut c_void);
            start_graphics_capture_with_item(window, config, ctx)
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
pub fn run_dxgi_capture(
    config: CaptureConfig,
    ctx: CaptureThreadContext,
) -> Result<(), CaptureError> {
    let monitor = match config.target {
        CaptureTarget::Primary => Monitor::primary().map_err(capture_unsupported)?,
        CaptureTarget::Monitor { monitor_index } => {
            Monitor::from_index(usize::try_from(monitor_index.saturating_add(1)).map_err(
                |err| CaptureError::TargetInvalid {
                    detail: err.to_string(),
                },
            )?)
            .map_err(|err| CaptureError::TargetInvalid {
                detail: err.to_string(),
            })?
        }
        CaptureTarget::Window { .. } => {
            return Err(CaptureError::TargetInvalid {
                detail: "DXGI duplication supports monitor targets only".to_owned(),
            });
        }
    };
    let mut api = DxgiDuplicationApi::new(monitor).map_err(|err| dxgi_error(&err))?;
    let timeout_ms = u32::try_from(config.min_update_interval_ms.max(1)).unwrap_or(u32::MAX);
    let mut frame_seq = 0_u64;

    while !ctx.stop.load(std::sync::atomic::Ordering::Relaxed) {
        match api.acquire_next_frame(timeout_ms) {
            Ok(frame) => {
                let captured = CapturedFrame {
                    texture: SendablePtr::new(frame.texture().clone()),
                    width: frame.width(),
                    height: frame.height(),
                    format: dxgi_format(frame.format()),
                    captured_at: std::time::Instant::now(),
                    frame_seq,
                    dirty_region: None,
                };
                push_frame(&ctx, captured)?;
                frame_seq = frame_seq.saturating_add(1);
            }
            Err(DxgiError::Timeout) => {
                thread::sleep(Duration::from_millis(config.min_update_interval_ms.max(1)));
            }
            Err(DxgiError::AccessLost) => {
                return Err(CaptureError::TargetLost {
                    detail: "DXGI output duplication access lost".to_owned(),
                });
            }
            Err(err) => return Err(dxgi_error(&err)),
        }
    }

    Ok(())
}
#[allow(clippy::needless_pass_by_value)]
fn start_graphics_capture_with_item<T>(
    item: T,
    config: CaptureConfig,
    ctx: CaptureThreadContext,
) -> Result<(), CaptureError>
where
    T: TryInto<windows_capture::settings::GraphicsCaptureItemType>,
{
    let settings = Settings::new(
        item,
        if config.cursor_visible {
            CursorCaptureSettings::WithCursor
        } else {
            CursorCaptureSettings::WithoutCursor
        },
        DrawBorderSettings::WithoutBorder,
        if config.secondary_windows {
            SecondaryWindowSettings::Include
        } else {
            SecondaryWindowSettings::Exclude
        },
        MinimumUpdateIntervalSettings::Custom(Duration::from_millis(
            config.min_update_interval_ms.max(1),
        )),
        if config.dirty_region_only {
            DirtyRegionSettings::ReportAndRender
        } else {
            DirtyRegionSettings::Default
        },
        ColorFormat::Bgra8,
        GraphicsHandlerFlags { ctx },
    );
    GraphicsHandler::start(settings).map_err(|err| CaptureError::ThreadFailed {
        detail: err.to_string(),
    })
}

struct GraphicsHandlerFlags {
    ctx: CaptureThreadContext,
}

struct GraphicsHandler {
    ctx: CaptureThreadContext,
    frame_seq: u64,
}

impl GraphicsCaptureApiHandler for GraphicsHandler {
    type Flags = GraphicsHandlerFlags;
    type Error = CaptureError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self {
            ctx: ctx.flags.ctx,
            frame_seq: 0,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.ctx.stop.load(std::sync::atomic::Ordering::Relaxed) {
            control.stop();
            return Ok(());
        }

        let captured = CapturedFrame {
            texture: SendablePtr::new(frame.as_raw_texture().clone()),
            width: frame.width(),
            height: frame.height(),
            format: match frame.color_format() {
                ColorFormat::Bgra8 => DxgiFormat::Bgra8,
                ColorFormat::Rgba8 => DxgiFormat::Rgba8,
                ColorFormat::Rgba16F => DxgiFormat::Rgba16F,
            },
            captured_at: std::time::Instant::now(),
            frame_seq: self.frame_seq,
            dirty_region: union_dirty_regions(&frame.dirty_regions().unwrap_or_default()),
        };
        push_frame(&self.ctx, captured)?;
        self.frame_seq = self.frame_seq.saturating_add(1);
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        self.ctx
            .stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

fn dxgi_error(err: &DxgiError) -> CaptureError {
    CaptureError::GraphicsApiUnsupported {
        detail: err.to_string(),
    }
}

const fn dxgi_format(format: DxgiDuplicationFormat) -> DxgiFormat {
    match format {
        DxgiDuplicationFormat::Rgba16F => DxgiFormat::Rgba16F,
        DxgiDuplicationFormat::Rgb10A2 => DxgiFormat::Rgb10A2,
        DxgiDuplicationFormat::Rgb10XrA2 => DxgiFormat::Rgb10XrA2,
        DxgiDuplicationFormat::Rgba8 => DxgiFormat::Rgba8,
        DxgiDuplicationFormat::Rgba8Srgb => DxgiFormat::Rgba8Srgb,
        DxgiDuplicationFormat::Bgra8 => DxgiFormat::Bgra8,
        DxgiDuplicationFormat::Bgra8Srgb => DxgiFormat::Bgra8Srgb,
    }
}

fn union_dirty_regions(regions: &[DirtyRegion]) -> Option<Rect> {
    let first = regions.first()?;
    let mut left = first.x;
    let mut top = first.y;
    let mut right = first.x.saturating_add(first.width);
    let mut bottom = first.y.saturating_add(first.height);

    for region in &regions[1..] {
        left = left.min(region.x);
        top = top.min(region.y);
        right = right.max(region.x.saturating_add(region.width));
        bottom = bottom.max(region.y.saturating_add(region.height));
    }

    Some(Rect {
        x: left,
        y: top,
        w: right.saturating_sub(left),
        h: bottom.saturating_sub(top),
    })
}
