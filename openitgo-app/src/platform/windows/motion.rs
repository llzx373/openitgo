use std::ffi::c_void;

#[link(name = "user32")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, param: u32, value: *mut c_void, flags: u32) -> i32;
}

pub fn animations_disabled() -> bool {
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    let mut enabled = 1_i32;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            &mut enabled as *mut i32 as *mut c_void,
            0,
        )
    };
    ok == 0 || enabled == 0
}
