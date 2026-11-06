#Requires -Version 7
# Real-Excel test of typing with cell autocomplete, in a blank workbook that is never saved.
# usage: pwsh -NoProfile -File test_excel.ps1 <govn.exe> <sendkeys.exe> <auto|never>
param([string]$Govn, [string]$SendKeys, [string]$Fix)

[Console]::OutputEncoding = [Text.Encoding]::UTF8
$cases = @(
    @('gox', 'gõ'),
    @('gooj', 'gộ'),
    @('gos', 'gó'),
    @('goxx', 'gox'),
    @('gox tieengs vieetj', 'gõ tiếng việt'),
    @('gox{BS}', 'g'),
    @('tieengs{BS}{BS}', 'tiế'),
    @('xin chaof', 'xin chào')
)

$before = @(Get-Process EXCEL -ErrorAction SilentlyContinue).Id
$xl = New-Object -ComObject Excel.Application
$kb = $null
$failed = 0
try {
    $xl.Visible = $true
    $xl.DisplayAlerts = $false
    $wb = $xl.Workbooks.Add()
    $ws = $wb.Worksheets.Item(1)
    $proc = Get-Process EXCEL | Where-Object { $before -notcontains $_.Id } | Select-Object -First 1
    if (-not $proc) { throw 'Could not identify the new Excel process' }

    $kb = Start-Process -FilePath $Govn -PassThru -WindowStyle Hidden -ArgumentList @(
        '--only-pid', $proc.Id, '--accept-injected', '--duration-secs', '90', '--suggestion-fix', $Fix)
    Start-Sleep -Milliseconds 500

    $col = 1
    foreach ($c in $cases) {
        # Autocomplete offers the entry above, so "go" shows as go|ogle
        $ws.Cells.Item(1, $col).Value2 = 'google test'
        $ws.Cells.Item(2, $col).Select() | Out-Null
        & $SendKeys $proc.Id ($c[0] + '{ENTER}')
        if ($LASTEXITCODE -ne 0) { throw "sendkeys exit $LASTEXITCODE" }
        Start-Sleep -Milliseconds 300
        $got = [string]$ws.Cells.Item(2, $col).Value2
        if ($got -ceq $c[1]) {
            "ok    '$($c[0])': '$got'"
        } else {
            $failed++
            "FAIL  '$($c[0])': muốn '$($c[1])', ra '$got'"
        }
        $col++
    }
} finally {
    if ($kb) { Stop-Process -Id $kb.Id -Force -ErrorAction SilentlyContinue }
    if ($wb) { $wb.Close($false) }
    $xl.Quit()
    [Runtime.InteropServices.Marshal]::ReleaseComObject($xl) | Out-Null
}
"thất bại: $failed"
exit ([int]($failed -ne 0))
