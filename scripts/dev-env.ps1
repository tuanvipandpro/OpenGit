# Dot-source this file to use the optional repository-local Windows build tools.
# An existing system toolchain is left available when .tools has not been provisioned.
$opengitRoot = Split-Path -Parent $PSScriptRoot
$opengitTools = Join-Path $opengitRoot '.tools'
if (-not $env:CARGO_BUILD_JOBS) {
    $env:CARGO_BUILD_JOBS = '4'
}
if (Test-Path -LiteralPath (Join-Path $opengitTools 'cargo\bin\cargo.exe')) {
    $env:CARGO_HOME = Join-Path $opengitTools 'cargo'
    $env:RUSTUP_HOME = Join-Path $opengitTools 'rustup'
    $env:PATH = (Join-Path $env:CARGO_HOME 'bin') + ';' + $env:PATH
}
$opengitMsvcBase = Join-Path $opengitTools 'msvc\VC\Tools\MSVC'
$opengitSdkBase = Join-Path $opengitTools 'msvc\Windows Kits\10'
if ((Test-Path -LiteralPath $opengitMsvcBase) -and (Test-Path -LiteralPath $opengitSdkBase)) {
    $opengitMsvc = Get-ChildItem -LiteralPath $opengitMsvcBase -Directory | Sort-Object Name -Descending | Select-Object -First 1
    $opengitSdk = Get-ChildItem -LiteralPath (Join-Path $opengitSdkBase 'Lib') -Directory | Sort-Object Name -Descending | Select-Object -First 1
    if ($opengitMsvc -and $opengitSdk) {
        $opengitCompiler = Join-Path $opengitMsvc.FullName 'bin\Hostx64\x64'
        $opengitSdkBin = Join-Path $opengitSdkBase ('bin\' + $opengitSdk.Name + '\x64')
        $opengitSdkInclude = Join-Path $opengitSdkBase ('Include\' + $opengitSdk.Name)
        $env:PATH = $opengitCompiler + ';' + $opengitSdkBin + ';' + $env:PATH
        $opengitIncludePaths = @((Join-Path $opengitMsvc.FullName 'include')) + @(@('ucrt','shared','um','winrt','cppwinrt') | ForEach-Object { Join-Path $opengitSdkInclude $_ })
        $env:INCLUDE = $opengitIncludePaths -join ';'
        $env:LIB = (Join-Path $opengitMsvc.FullName 'lib\x64') + ';' + (Join-Path $opengitSdk.FullName 'ucrt\x64') + ';' + (Join-Path $opengitSdk.FullName 'um\x64')
        $env:VCToolsInstallDir = $opengitMsvc.FullName + '\'
        $env:VCToolsVersion = $opengitMsvc.Name
        $env:WindowsSdkDir = $opengitSdkBase + '\'
        $env:WindowsSDKVersion = $opengitSdk.Name + '\'
        $env:VSCMD_ARG_HOST_ARCH = 'x64'
        $env:VSCMD_ARG_TGT_ARCH = 'x64'
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $opengitCompiler 'link.exe'
    }
}
