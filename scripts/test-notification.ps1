param(
    [string]$Title = "Test Title",
    [string]$Body  = "Test Body"
)

$ps5 = "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe"
if ($PSVersionTable.PSEdition -eq 'Core') {
    & $ps5 -ExecutionPolicy Bypass -File "$PSCommandPath" -Title $Title -Body $Body
    exit
}

$null = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime]
$null = [Windows.UI.Notifications.ToastNotification, Windows.UI.Notifications, ContentType=WindowsRuntime]
$null = [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType=WindowsRuntime]

$xml = [Windows.Data.Xml.Dom.XmlDocument]::new()
$xml.LoadXml("<toast><visual><binding template='ToastGeneric'><text>$Title</text><text>$Body</text></binding></visual></toast>")

$toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier("Microsoft.Windows.Explorer").Show($toast)

Write-Host "Sent: [$Title] $Body"
