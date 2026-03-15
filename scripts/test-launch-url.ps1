# launch_url 付きトースト通知を送信し、notifbar でのURL起動を手動テストするスクリプト。
#
# 使い方:
#   .\test-launch-url.ps1 -Scheme https
#   .\test-launch-url.ps1 -Scheme slack
#   .\test-launch-url.ps1 -Scheme chrome
#   .\test-launch-url.ps1 -Url "slack://channel/CXXXXXXXX"
#
# PowerShell 7+ (pwsh) から実行した場合は自動的に PS5 に切り替える（WinRT API のため）。

param(
    [ValidateSet("https", "slack", "chrome", "mailto", "ms-store", "custom")]
    [string]$Scheme = "https",

    # -Scheme custom のときに使うURL（任意のスキームを直接指定）
    [string]$Url = ""
)

# WinRT は PowerShell 5 でのみ動作するため、pwsh から呼ばれた場合は PS5 に転送する
$ps5 = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
if ($PSVersionTable.PSEdition -eq 'Core') {
    & $ps5 -ExecutionPolicy Bypass -File "$PSCommandPath" -Scheme $Scheme -Url $Url
    exit
}

# スキームごとのテスト用URLとタイトル
$schemes = @{
    "https"    = @{ Url = "https://www.google.com"; Label = "Chrome / デフォルトブラウザ" }
    "slack"    = @{ Url = "slack://open";            Label = "Slack アプリ" }
    "chrome"   = @{ Url = "googlechrome://";         Label = "Google Chrome" }
    "mailto"   = @{ Url = "mailto:test@example.com"; Label = "メールクライアント" }
    "ms-store" = @{ Url = "ms-windows-store://home"; Label = "Microsoft Store" }
    "custom"   = @{ Url = $Url;                      Label = "カスタムURL: $Url" }
}

if ($Scheme -eq "custom" -and $Url -eq "") {
    Write-Error "-Scheme custom を使う場合は -Url でURLを指定してください。"
    exit 1
}

$target = $schemes[$Scheme]
$launchUrl = $target.Url
$label = $target.Label

Write-Host ""
Write-Host "=== launch URL テスト ===" -ForegroundColor Cyan
Write-Host "スキーム : $Scheme"
Write-Host "URL      : $launchUrl"
Write-Host "対象     : $label"
Write-Host ""

# 1. cmd /c start で直接起動確認（open_url() と同じ動作）
Write-Host "[1] cmd /c start で直接起動..." -ForegroundColor Yellow
$proc = Start-Process -FilePath "cmd" -ArgumentList "/c", "start", "", $launchUrl `
    -WindowStyle Hidden -PassThru
Start-Sleep -Milliseconds 500
if ($proc.HasExited -or $proc.Id) {
    Write-Host "    OK: プロセス起動成功" -ForegroundColor Green
} else {
    Write-Host "    NG: プロセス起動失敗" -ForegroundColor Red
}

# 2. launch_url 付きトースト通知を送信（notifbar でのクリック動作を確認）
Write-Host ""
Write-Host "[2] launch_url 付きトースト通知を送信..." -ForegroundColor Yellow

$null = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime]
$null = [Windows.UI.Notifications.ToastNotification, Windows.UI.Notifications, ContentType=WindowsRuntime]
$null = [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType=WindowsRuntime]

$escapedUrl = [System.Security.SecurityElement]::Escape($launchUrl)
$xmlStr = "<toast launch=`"$escapedUrl`"><visual><binding template='ToastGeneric'>" +
          "<text>launch URL テスト ($Scheme)</text>" +
          "<text>クリックすると起動: $launchUrl</text>" +
          "</binding></visual></toast>"

$xml = [Windows.Data.Xml.Dom.XmlDocument]::new()
$xml.LoadXml($xmlStr)
$toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier("Microsoft.Windows.Explorer").Show($toast)

Write-Host "    OK: トースト送信完了" -ForegroundColor Green
Write-Host ""
Write-Host "notifbar の通知カードをクリックして '$label' が起動することを確認してください。" -ForegroundColor Cyan
Write-Host ""
