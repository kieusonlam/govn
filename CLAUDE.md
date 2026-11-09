# govn

Bộ gõ tiếng Việt cho Windows bằng Rust. Engine port từ `ukengine` của UniKey (C++). README.md mô tả cách dùng, build và test; file này chỉ ghi những gì đọc code không thấy.

## Cấu trúc

- `engine/` (`govn-engine`): engine gõ, không phụ thuộc Windows. `engine/src/tables.rs` sinh bằng `tools/gen_tables.py`, không sửa tay.
- `app/` (`govn`): file exe.
  - `main.rs`: hook bàn phím `WH_KEYBOARD_LL`, gõ lại bằng `SendInput`, theo dõi cửa sổ foreground (`SetWinEventHook`), phím tắt.
  - `tray.rs`: icon và menu khay. `settings.rs` + `ui/settings.slint`: cửa sổ Cài đặt.
  - `config.rs`: `settings.ini` và `apps.ini`. `admin.rs`: Task Scheduler cho chế độ admin. `update.rs`: hỏi GitHub bản mới.
- `tools/`: script test thật (Chrome, VS Code, Excel, admin), so sánh với bản C++ (`difftest.py`, `oracle/`), `third_party.py`.
- `dist/`: zip các bản đã phát hành (không commit).

## Hai tiến trình

Cửa sổ Cài đặt là tiến trình riêng: `govn.exe --settings --config <ini> --task-name <t> --autostart-key <k>`. Phần gõ phím không nạp Slint, đừng gọi Slint từ code chạy trong hook.

- Cả hai đều ghi `settings.ini`, nên mọi thay đổi là đọc-sửa-ghi (`config::Settings::load` → sửa → `save`, ghi qua file tạm).
- Ghi xong, cửa sổ Cài đặt gửi `WM_RELOAD` tới cửa sổ ẩn `WINDOW_CLASS` của tiến trình gõ phím. `ChangeWindowMessageFilterEx` cho phép tin này đi qua khi tiến trình gõ phím chạy quyền admin.
- `apps.ini` (nhớ V/E theo ứng dụng) chỉ tiến trình gõ phím ghi.
- Bật/tắt admin vẫn do tiến trình gõ phím làm (`WM_COMMAND CMD_ADMIN`), vì nó lo UAC và khởi động lại.
- File cũ hoặc sửa tay phải luôn đọc được: khoá lạ và giá trị sai bị bỏ qua, không làm hỏng phần còn lại.

## Ràng buộc khi sửa

- **Chỉ có Telex và VNI, không có kiểu gõ tắt.** Telex của govn là `InputMethod::SimpleTelex` của engine (`config::TELEX`): gõ đầy đủ như trên điện thoại (`uw` → ư, `ow` → ơ); `w` đứng riêng và `[ ]` giữ nguyên. Telex/SimpleTelex2 của engine không đưa ra giao diện; file cài đặt cũ ghi `simple`, `simple2` đều đọc thành Telex.
- **Menu khay giữ đủ mọi tùy chọn** (người dùng muốn ai thích chỉnh gì thì chỉnh ngay ở đó); cửa sổ Cài đặt là chỗ rộng hơn cho cùng các tùy chọn, cộng danh sách ứng dụng và phím tắt.
- **Không thêm gõ tắt** và các tính năng phụ kiểu đó; người dùng đã quyết định giữ app gọn.
- **Mặc định:** Telex, bỏ dấu kiểu cũ, phím tắt Ctrl+Shift. Chỉ đổi khi người dùng yêu cầu, vì đổi mặc định là đổi hành vi với người mới cài.
- **Mạng:** `update.rs` là kết nối mạng duy nhất và chỉ chạy khi người dùng bấm. Không gửi gì liên quan tới phím đã gõ, không tự tải hay tự cài.
- **Engine:** chuỗi phím bất kỳ không được panic. Khi đổi logic engine, chạy `tools/difftest.py` so với bản C++ gốc.

## Build

- Build release lần đầu mất 10-15 phút (Slint). `cargo build --release -p govn`.
- Windows Defender đã nhiều lần chặn bản build nằm trong `%TEMP%` (`Trojan:Win32/Bearfoos.A!ml`, `Behavior:Win32/Execution.A!ml`) vì app đọc và gõ lại phím. Nếu build hoặc chạy bị chặn thì báo cho người dùng. **Không thêm exclusion cho Defender và không tìm cách né phát hiện.**
- Icon app ("Gõ" trắng trên nền đỏ) vẽ bằng `pwsh -NoProfile -File tools/make_icon.ps1`, ra `app/govn.ico`, `app/ui/icon.png`, `docs/icon.png`. Không sửa tay các file đó. Icon V/E ở khay vẽ lúc chạy trong `tray.rs`.
- Phiên bản nằm ở 3 chỗ, phải đổi cùng nhau: `Cargo.toml` (`[workspace.package] version`), `app/govn.rc` (`FILEVERSION`, `PRODUCTVERSION`, hai chuỗi `"x.y.z"`), `app/govn.manifest`.

## Test

- `cargo test --workspace`: luôn chạy được. Có một test gọi GitHub thật bị `#[ignore]`.
- **e2e** (`cargo run --release -p govn --example e2e -- <govn.exe>`) gõ phím thật và cần cửa sổ test ở foreground khoảng 1 phút:
  - Hỏi người dùng thoát govn đang chạy trước, nếu không phím bị xử lý hai lần.
  - Trong lúc chạy người dùng không gõ hay bấm được, nên hỏi trước khi chạy.
  - "mất focus" nghĩa là có cửa sổ khác lên trước; chạy lại, không phải lỗi code.
  - Test dùng file cài đặt tạm, task `govn-e2e-test` và khoá registry `Software\govn-e2e-test\Run`, không đụng cài đặt thật.
- Các script trong `tools/test_*.py` mở ứng dụng thật với hồ sơ tạm. `test_admin*.py` cần người dùng bấm UAC.

## An toàn trên máy người dùng

- Coi govn và EVKey đang cài trên máy là chỉ đọc: không sửa cài đặt, task hay registry thật của chúng. Test luôn dùng `--config`, `--task-name` và `--autostart-key` riêng.
- **Không chụp màn hình người dùng.** Muốn xem giao diện thì mở cửa sổ test của chính govn với config tạm rồi chụp riêng cửa sổ đó bằng `PrintWindow` (chỉ lấy nội dung của cửa sổ đó, không lấy cái nằm đè lên).

## Quy ước

- Comment và commit bằng tiếng Anh; README, giao diện và release notes bằng tiếng Việt.
- Script mới viết bằng Python (`sys.stdout.reconfigure(encoding="utf-8")` nếu in tiếng Việt) hoặc PowerShell 7 (`pwsh`).
- Nhiều file dùng CRLF. Khi sửa bằng script, giữ nguyên kiểu xuống dòng của file.
- Phát hành: dùng skill `/release` (`.claude/skills/release/SKILL.md`).
- Giấy phép: mã govn GPL-3.0-or-later; vì Slint chỉ cho dùng theo GPL-3.0 nên `govn.exe` phát hành theo GPL-3.0. Không thêm thư viện có giấy phép không tương thích GPL-3.0.
