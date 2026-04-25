# Build the project
Write-Host "Building RuneScript Compiler..."
cargo build --release

# Create installation directory
$INSTALL_DIR = "$env:USERPROFILE\.rsc"
New-Item -ItemType Directory -Force -Path "$INSTALL_DIR\bin" | Out-Null

# Copy the binary
Write-Host "Installing RuneScript Compiler..."
$TARGET = "$INSTALL_DIR\bin\rsc.exe"
$MAX_RETRIES = 3
$RETRY_WAIT = 2

# Try to copy with retries
for ($i = 1; $i -le $MAX_RETRIES; $i++) {
    try {
        # If file exists and is in use, try to stop the process
        if (Test-Path $TARGET) {
            $processes = Get-Process | Where-Object {$_.Path -eq $TARGET}
            if ($processes) {
                Write-Host "Stopping existing RSC processes..."
                $processes | ForEach-Object { $_.Kill() }
                Start-Sleep -Seconds 1
            }
        }
        
        Copy-Item "target\release\runescript-compiler.exe" -Destination $TARGET -Force
        break
    }
    catch {
        if ($i -eq $MAX_RETRIES) {
            Write-Host "Error: Could not install RSC after $MAX_RETRIES attempts. Please close any running instances and try again."
            exit 1
        }
        Write-Host "Installation attempt $i failed. Retrying in $RETRY_WAIT seconds..."
        Start-Sleep -Seconds $RETRY_WAIT
    }
}

# Add to PATH
$USER_PATH = [Environment]::GetEnvironmentVariable("PATH", "User")
if ($USER_PATH -notlike "*$INSTALL_DIR\bin*") {
    [Environment]::SetEnvironmentVariable(
        "PATH",
        "$USER_PATH;$INSTALL_DIR\bin",
        "User"
    )
}

# Create PowerShell profile if it doesn't exist
if (!(Test-Path -Path $PROFILE)) {
    New-Item -ItemType File -Path $PROFILE -Force | Out-Null
}

# Add alias to PowerShell profile
$ALIAS_LINE = "Set-Alias -Name rsc -Value '$INSTALL_DIR\bin\rsc.exe'"
if (!(Select-String -Path $PROFILE -Pattern "Set-Alias.*rsc.*" -Quiet)) {
    Add-Content $PROFILE "`n# RuneScript Compiler"
    Add-Content $PROFILE $ALIAS_LINE
}

Write-Host "Installation complete! Please restart your terminal to use 'rsc' command." 