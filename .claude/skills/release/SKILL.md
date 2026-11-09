---
name: release
description: Phát hành một bản govn mới lên GitHub (nâng phiên bản, build, đóng gói zip kèm giấy phép, tag, release notes). Dùng khi người dùng nói "release", "phát hành", "ra bản mới".
---

# Phát hành govn

Chỉ làm khi người dùng yêu cầu phát hành. Push, tag và tạo release đều công khai, nên mỗi bước đó cần lệnh rõ ràng từ người dùng trong lượt hiện tại.

## 1. Chọn phiên bản
- Sửa lỗi: tăng số cuối (0.2.0 → 0.2.1). Thêm tính năng: tăng số giữa (0.2.x → 0.3.0). Nói rõ phiên bản đã chọn trong câu trả lời.
- Đổi ở **cả 3 chỗ**:
  - `Cargo.toml` (`[workspace.package] version`).
  - `app/govn.rc`: `FILEVERSION a,b,c,0`, `PRODUCTVERSION a,b,c,0` và hai chuỗi `"a.b.c"`.
  - `app/govn.manifest` (`version="a.b.c.0"`).
- `update::CURRENT_VERSION` lấy từ Cargo nên tự đổi theo.

## 2. Build và kiểm tra
```bash
cargo test --release --workspace
cargo build --release -p govn
```
- Kiểm tra `FileVersion` của exe đúng phiên bản mới: `(Get-Item govn.exe).VersionInfo` trong `pwsh`.
- e2e gõ phím thật và chiếm bàn phím khoảng 1 phút: hỏi người dùng trước, và họ phải thoát govn đang chạy (xem CLAUDE.md). Không chạy được thì ghi vào mục "Chưa kiểm tra".
- Nếu Defender chặn file build: dừng lại và báo người dùng. Không thêm exclusion.

## 3. Đóng gói
- Zip tên `govn-<ver>-windows-x64.zip`, đặt trong `dist/` (đã có trong `.gitignore`). Gồm đúng 5 file ở gốc zip:
  - `govn.exe`
  - `LICENSE`
  - `NOTICE`
  - `README.md`
  - `THIRD-PARTY-LICENSES.txt`, tạo bằng `python tools/third_party.py <đường dẫn>`
- Tạo zip bằng `zipfile` của Python, không dùng `Compress-Archive`: PowerShell 5.1 ghi tên entry bằng `\`.
- Ghi lại SHA-256 của `govn.exe` và của file zip.

## 4. Commit, tag, push
- Commit thay đổi phiên bản riêng, message `Release <ver>`.
- Đưa vào `main` bằng fast-forward. Tag có chú thích `v<ver>` (`-m "govn <ver>"`), rồi `git push origin main v<ver>`.
- README phải được cập nhật **trước** tag, vì README trong zip lấy từ đúng commit đó.

## 5. GitHub release
```bash
gh release create v<ver> dist/govn-<ver>-windows-x64.zip --title "govn <ver>" --notes-file <notes.md> --prerelease --verify-tag
```
- Vẫn để `--prerelease` cho tới khi người dùng nói khác.
- Release notes bằng tiếng Việt. Lấy bố cục của bản trước (`gh release view v<bản trước>`) gồm các mục:
  - Có gì mới
  - Cài đặt hoặc cập nhật
  - Cần biết trước khi dùng (chưa ký số, Defender, chế độ admin)
  - Đã kiểm tra ở bản này
  - Chưa kiểm tra ở bản này
  - Lỗi đã biết
  - Giấy phép
  - SHA-256
- Chỉ ghi "đã kiểm tra" những gì đã thật sự chạy trên đúng bản này. Cái gì chạy trên bản trước đó thì nói rõ.
- Tải lại file zip từ release (`gh release download`) và so SHA-256 với bản trên máy.
