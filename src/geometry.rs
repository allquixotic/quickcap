use anyhow::{Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn from_points(a: (i32, i32), b: (i32, i32)) -> Option<Self> {
        let width = a.0.abs_diff(b.0);
        let height = a.1.abs_diff(b.1);
        (width >= 2 && height >= 2).then_some(Self {
            x: a.0.min(b.0), y: a.1.min(b.1), width, height,
        })
    }

    pub fn output_size(self) -> Result<(u32, u32)> {
        ensure!(self.width >= 2 && self.height >= 2, "Capture area must be at least 2×2 pixels");
        ensure!(self.width <= 32768 && self.height <= 32768, "Capture area too large");
        let scale = (1920.0 / self.width as f64).min(1080.0 / self.height as f64).min(1.0);
        let width = ((self.width as f64 * scale) as u32) & !1;
        let height = ((self.height as f64 * scale) as u32) & !1;
        ensure!(width >= 2 && height >= 2, "Capture area too narrow after downscaling");
        Ok((width, height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scale_caps_without_upscaling() {
        for (w,h,expected) in [(3840,2160,(1920,1080)), (2560,1600,(1728,1080)), (1280,720,(1280,720)), (801,601,(800,600))] {
            assert_eq!(Rect{x:0,y:0,width:w,height:h}.output_size().unwrap(),expected);
        }
    }
    #[test]
    fn selection_handles_negative_origins_and_reverse_drags() {
        assert_eq!(Rect::from_points((20,40),(-100,-60)),Some(Rect{x:-100,y:-60,width:120,height:100}));
        assert!(Rect::from_points((2,2),(2,50)).is_none());
    }
    #[test]
    fn pathological_geometry_is_rejected() {
        assert!(Rect{x:0,y:0,width:1,height:100}.output_size().is_err());
        assert!(Rect{x:0,y:0,width:u32::MAX,height:100}.output_size().is_err());
    }
}
