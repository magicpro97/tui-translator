# Hướng dẫn setup GitHub Actions self-hosted runner trên Windows

**Mục đích:** Chạy `cargo test` trên máy Windows thật để verify fix STATUS_ACCESS_VIOLATION từ macOS.
Sau khi setup, mỗi lần push branch → runner tự chạy → log xem được từ GitHub Actions UI.

---

## Yêu cầu máy Windows

- Windows 10/11 (64-bit)
- Quyền Administrator
- Kết nối internet ra github.com (cổng 443)
- RAM ≥ 8 GB (Rust test build nặng)
- Disk trống ≥ 20 GB cho `target/` + cargo cache

---

## Bước 1 — Cài Rust toolchain (chỉ làm 1 lần)

Mở **PowerShell as Administrator**:

```powershell
# Cài Visual Studio Build Tools 2022 (cần cho MSVC linker + Windows SDK)
# Tải từ: https://aka.ms/vs/17/release/vs_BuildTools.exe
# Chạy installer, chọn workload "Desktop development with C++"
# Tick: MSVC v143, Windows 11 SDK, C++ CMake tools

# Cài Rust
winget install Rustlang.Rustup
# Hoặc tải từ: https://win.rustup.rs/x86_64

# Verify
rustc --version     # phải >= 1.88
cargo --version
```

---

## Bước 2 — Tạo thư mục runner + tải binary

```powershell
# Tạo thư mục (đặt ổ C nếu muốn, hoặc D:\actions-runner)
mkdir C:\actions-runner
cd C:\actions-runner

# Tải runner (version mới nhất ở https://github.com/actions/runner/releases)
Invoke-WebRequest -Uri https://github.com/actions/runner/releases/download/v2.319.1/actions-runner-win-x64-2.319.1.zip -OutFile runner.zip
Expand-Archive runner.zip -DestinationPath .
Remove-Item runner.zip

# Verify
.\config.cmd --help
```

---

## Bước 3 — Lấy registration token từ GitHub

**Trên máy macOS** (của tui), chạy để gen token:

```bash
gh api -X POST repos/magicpro97/tui-translator/actions/runners/registration-token --jq '.token'
```

Copy token đó (dài ~30 ký tự, expires trong 1 giờ).

---

## Bước 4 — Đăng ký runner

**Trên máy Windows**, quay lại PowerShell tại `C:\actions-runner`:

```powershell
.\config.cmd --url https://github.com/magicpro97/tui-translator --token <PASTE_TOKEN_VAO_DAY>
```

Hỏi các câu:
- **Runner group:** nhấn Enter (default "Default")
- **Runner name:** đặt tên dễ nhớ, ví dụ `windows-linhn-desktop`
- **Labels:** nhấn Enter (default, hoặc thêm `windows-desktop,self-hosted`)
- **Work folder:** nhấn Enter (`_work`)

Sau khi xong sẽ thấy:
```
√ Runner successfully added
√ Runner connection is good
```

---

## Bước 5 — Cài runner như Windows Service (chạy nền, tự khởi động)

```powershell
# Service tự start khi Windows boot, tự restart nếu crash
.\svc.cmd install
.\svc.cmd start

# Verify
Get-Service "actions.runner.*"   # phải Status = Running
```

Sau bước này runner đã live. Check trên GitHub:
```
https://github.com/magicpro97/tui-translator/settings/actions/runners
```
Phải thấy `windows-linhn-desktop` với status "Idle" + label "self-hosted, windows, X64".

---

## Bước 6 — Sửa workflow để dùng self-hosted runner (tui sẽ làm trên macOS)

Cần thêm 1 job mới vào `.github/workflows/ci.yml` (hoặc tạo workflow riêng) chạy trên `runs-on: self-hosted`. Khi push branch `fix/windows-com-teardown`, job sẽ chạy trên máy Windows của user.

Tui sẽ tạo file `.github/workflows/windows-selfhosted-test.yml` sau khi user xác nhận runner đã live.

---

## Bước 7 — Test thử runner

Trên máy macOS:
```bash
# Tạo branch test
git checkout -b test/runner-alive
echo "# runner test $(date)" >> README.md
git add README.md
git commit -m "test: verify self-hosted runner is alive"
git push origin test/runner-alive

# Check Actions tab
gh run list --workflow=windows-selfhosted-test.yml --limit 1
```

Trên máy Windows, mở `C:\actions-runner\_diag\Runner-*.log` để xem log realtime nếu job chạy.

---

## Troubleshooting

**Runner không connect được:**
```powershell
# Check service
Get-Service "actions.runner.*" | Format-List
# Check log
Get-Content C:\actions-runner\_diag\Runner-*.log -Tail 50
# Restart
.\svc.cmd stop
.\svc.cmd start
```

**Cargo test fail với linker error:**
- Visual Studio Build Tools chưa cài đúng workload
- Chạy lại installer, đảm bảo "Desktop development with C++" được chọn
- Verify: `cl.exe` phải có trong PATH sau khi mở "Developer Command Prompt for VS 2022"

**Test pass trên self-hosted nhưng vẫn crash trên hosted runner:**
- 2 môi trường khác nhau. Hosted runner thiếu audio driver (skip WASAPI test), self-hosted đầy đủ.
- Nếu self-hosted PASS, fix đã work. Hosted runner crash là dấu hiệu cần test stub cho WASAPI.

**Muốn gỡ runner:**
```powershell
.\svc.cmd stop
.\svc.cmd uninstall
cd ..
Remove-Item -Recurse C:\actions-runner
# Trên GitHub: Settings → Actions → Runners → Remove
```

---

## Khi xong

Báo lại cho tui:
1. Runner name đã đặt (để tui update workflow label)
2. Service status (phải Running)
3. GitHub Settings → Actions → Runners có hiện không

Tui sẽ tạo workflow `.github/workflows/windows-selfhosted-test.yml` dùng `runs-on: self-hosted` để chạy fix verify.
