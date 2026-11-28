//! Key codes and modifier bits for saved shortcuts. Settings store the Windows
//! virtual-key and RegisterHotKey values on every platform, so a settings file
//! means the same thing everywhere; Linux translates them to keysyms.
pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
#[allow(dead_code)]
pub const MOD_WIN: u32 = 0x8;
pub const VK_SNAPSHOT: u32 = 0x2C;
