use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = pico_args::Arguments::from_env();
    let mode: String = args.value_from_str("--mode")?;
    let out: String = args.value_from_str("--out")?;
    let out_path = PathBuf::from(out);

    match mode.as_str() {
        "full" => run_full(&out_path)?,
        "region" => run_region(&out_path)?,
        _ => return Err("unknown mode".into()),
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn run_full(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("screencapture").arg("-x").arg(path).status()?;
    if !status.success() {
        return Err("screencapture failed".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_region(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("screencapture")
        .arg("-i")
        .arg("-x")
        .arg(path)
        .status()?;
    if !status.success() {
        return Err("screencapture failed".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn run_full(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    // Prefer grim; fallback to gnome-screenshot
    if Command::new("grim")
        .arg(path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(());
    }
    let status = Command::new("gnome-screenshot")
        .arg("-f")
        .arg(path)
        .status()?;
    if !status.success() {
        return Err("No supported screenshot tool found (install grim or gnome-screenshot)".into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn run_region(path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(selection) = Command::new("slurp").output() {
        if selection.status.success() {
            let geometry = String::from_utf8_lossy(&selection.stdout)
                .trim()
                .to_string();
            if !geometry.is_empty()
                && Command::new("grim")
                    .arg("-g")
                    .arg(geometry)
                    .arg(path)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            {
                return Ok(());
            }
        }
    }

    let status = Command::new("gnome-screenshot")
        .arg("-a")
        .arg("-f")
        .arg(path)
        .status()?;
    if !status.success() {
        return Err(
            "No supported region capture tool found (install grim+slurp or gnome-screenshot)"
                .into(),
        );
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_full(_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Stdio;

    let path = _path.to_string_lossy();
    // Captura todos los monitores (virtual screen) en lugar de solo el primario
    let script = format!(
        r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

try {{
    # Virtual screen bounds = todos los monitores combinados
    $left = [System.Windows.Forms.SystemInformation]::VirtualScreen.Left
    $top = [System.Windows.Forms.SystemInformation]::VirtualScreen.Top
    $width = [System.Windows.Forms.SystemInformation]::VirtualScreen.Width
    $height = [System.Windows.Forms.SystemInformation]::VirtualScreen.Height
    
    $bitmap = New-Object System.Drawing.Bitmap $width, $height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($left, $top, 0, 0, $bitmap.Size)
    $bitmap.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Png)
    
    # Liberar recursos
    $graphics.Dispose()
    $bitmap.Dispose()
    exit 0
}} catch {{
    exit 1
}}
"#,
        path = path.replace("\\", "\\\\").replace("'", "''")
    );

    let status = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    if !status.success() {
        return Err("Windows PowerShell capture failed".into());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_region(_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Stdio;

    let path = _path
        .to_string_lossy()
        .replace("\\", "\\\\")
        .replace("'", "''");
    let script = format!(
        r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

# Limpiar el clipboard antes de capturar para evitar guardar imagen anterior
[System.Windows.Forms.Clipboard]::Clear()

function Save-ClipImage([string]$p, [int]$timeoutMs) {{
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  while ($sw.ElapsedMilliseconds -lt $timeoutMs) {{
    Start-Sleep -Milliseconds 200
    if ([System.Windows.Forms.Clipboard]::ContainsImage()) {{
      $img = [System.Windows.Forms.Clipboard]::GetImage()
      if ($img) {{
        try {{
          $img.Save($p, [System.Drawing.Imaging.ImageFormat]::Png)
          $img.Dispose()
          return $true
        }} catch {{
          return $false
        }}
      }}
    }}
  }}
  return $false
}}

$started = $false

# Intentar ms-screenclip (Windows 10/11)
try {{
  Start-Process "ms-screenclip:" -ErrorAction Stop
  $started = $true
}} catch {{}}

# Fallback a Snipping Tool
if (-not $started) {{
  try {{
    Start-Process "snippingtool.exe" "/clip" -ErrorAction Stop
    $started = $true
  }} catch {{}}
}}

if (-not $started) {{ exit 2 }}

# Esperar a que el usuario capture (timeout 30 segundos)
if (-not (Save-ClipImage '{path}' 30000)) {{ exit 3 }}

exit 0
"#,
        path = path
    );

    let status = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-STA")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(&script)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;

    match status.code() {
        Some(0) => Ok(()),
        Some(2) => Err("No screen clipping tool found (ms-screenclip or snippingtool)".into()),
        Some(3) => Err("Timed out waiting for region capture (user cancelled?)".into()),
        _ => Err("Windows region capture failed".into()),
    }
}
