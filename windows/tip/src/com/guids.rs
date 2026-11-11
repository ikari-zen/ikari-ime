//! この TIP の GUID（2026-11-12 に乱数で生成した。変えると、登録済みの環境で別の IME として扱われる）。

use windows_core::GUID;

/// テキストサービスの COM のクラス（`HKCR\CLSID\{…}`、`DllGetClassObject` で照合する）
pub const CLSID_TEXT_SERVICE: GUID = GUID::from_u128(0x83a140be_decb_4358_867b_7b77d6541276);

/// 日本語の入力プロファイル（`ITfInputProcessorProfileMgr::RegisterProfile`）
pub const GUID_PROFILE_JAPANESE: GUID = GUID::from_u128(0x35eab916_5717_4cfa_8991_7264079fd1c1);
