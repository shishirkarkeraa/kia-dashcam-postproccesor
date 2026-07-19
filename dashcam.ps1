# Load the Windows component required to send files to the Recycle Bin
Add-Type -AssemblyName Microsoft.VisualBasic
function Send-ToRecycleBin([string]$FilePath) {
    if (Test-Path $FilePath) {
        $fullPath = (Get-Item $FilePath).FullName
        [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($fullPath, 'OnlyErrorDialogs', 'SendToRecycleBin')
    }
}

# ==========================================
# DASHBOARD & SILENT EXECUTION ENGINE
# ==========================================
$script:startTime = Get-Date
$script:globalCompletedTasks = 0
$script:totalTasks = 0

function Update-Dashboard($stageName, $stageTotal, $stageCompleted, $currentFile, $status, $statusColor) {
    $elapsed = (Get-Date) - $script:startTime
    
    $globalPct = 0
    if ($script:totalTasks -gt 0) { $globalPct = [math]::Round(($script:globalCompletedTasks / $script:totalTasks) * 100, 2) }
    
    $stagePct = 0
    if ($stageTotal -gt 0) { $stagePct = [math]::Round(($stageCompleted / $stageTotal) * 100, 2) }
    
    $etaString = "Calculating..."
    if ($script:globalCompletedTasks -gt 0) {
        $secondsPerTask = $elapsed.TotalSeconds / $script:globalCompletedTasks
        $remainingTasks = $script:totalTasks - $script:globalCompletedTasks
        $etaSeconds = $secondsPerTask * $remainingTasks
        $etaString = [timespan]::FromSeconds($etaSeconds).ToString("hh\:mm\:ss")
    }

    Clear-Host
    Write-Host "===============================================================" -ForegroundColor DarkCyan
    Write-Host "  _  _    _      _       ____            _                        " -ForegroundColor Cyan
    Write-Host " | |/ /  | |    / \     |  _ \  __ _ ___| |__   ___ __ _ _ __ ___ " -ForegroundColor Cyan
    Write-Host " | ' /   | |   / _ \    | | | |/ _' / __| '_ \ / __/ _' | '_ ' _ \" -ForegroundColor Cyan
    Write-Host " | . \   | |  / ___ \   | |_| | (_| \__ \ | | | (_| (_| | | | | | |" -ForegroundColor Cyan
    Write-Host " |_|\_\_ |_| /_/   \_\  |____/ \__,_|___/_| |_|\___\__,_|_| |_| |_|" -ForegroundColor Cyan
    Write-Host "                                                                 "
    Write-Host "                    POST-PROCESSING ENGINE                       " -ForegroundColor White
    Write-Host "                 Developed by Shishir Karkera                    " -ForegroundColor Yellow
    Write-Host "===============================================================" -ForegroundColor DarkCyan
    Write-Host " Directory:     $((Get-Location).Path)" -ForegroundColor Gray
    Write-Host " Stage:         $stageName" -ForegroundColor Yellow
    Write-Host " Working On:    $currentFile" -ForegroundColor Cyan
    Write-Host " Status:        $status" -ForegroundColor $statusColor
    Write-Host "---------------------------------------------------------------" -ForegroundColor DarkGray
    Write-Host " Time Elapsed:  $($elapsed.ToString("hh\:mm\:ss"))"
    Write-Host " Estimated ETA: $etaString"
    Write-Host "---------------------------------------------------------------" -ForegroundColor DarkGray
    Write-Host " Stage Files:   $stageCompleted / $stageTotal   ($stagePct % Stage Complete)"
    Write-Host " Total Tasks:   $($script:globalCompletedTasks) / $($script:totalTasks)   ($globalPct % Total Complete)"
    Write-Host "===============================================================" -ForegroundColor DarkCyan
}

function Invoke-SilentProcess($exe, $argArray, $failMessage) {
    $logFile = ".\temp_error_log.txt"
    $proc = Start-Process -FilePath $exe -ArgumentList $argArray -NoNewWindow -Wait -RedirectStandardError $logFile -PassThru
    
    if ($proc.ExitCode -ne 0) {
        Clear-Host
        Write-Host "===============================================================" -ForegroundColor Red
        Write-Host " FATAL ERROR DETECTED" -ForegroundColor White -BackgroundColor Red
        Write-Host "===============================================================" -ForegroundColor Red
        Write-Host $failMessage -ForegroundColor Yellow
        Write-Host "`n--- RAW CRASH LOG ---" -ForegroundColor Gray
        if (Test-Path $logFile) {
            Get-Content $logFile
        } else {
            Write-Host "[Error log could not be generated. Check if file exists.]" -ForegroundColor Red
        }
        Write-Host "---------------------" -ForegroundColor Gray
        Remove-Item $logFile -ErrorAction SilentlyContinue
        Write-Host "`nPress any key to exit..." -ForegroundColor Yellow
        $null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
        exit
    }
    Remove-Item $logFile -ErrorAction SilentlyContinue
}

# ==========================================
# PRE-CALCULATION & SETUP
# ==========================================
Update-Dashboard -stageName "0/4: Initializing Engine" -stageTotal 0 -stageCompleted 0 -currentFile "Scanning directory..." -status "Loading" -statusColor "Yellow"

if (-not (Test-Path .\temp_stacked)) { New-Item -ItemType Directory -Force -Path .\temp_stacked | Out-Null }
$listPath = ".\concat_list.txt"
New-Item -ItemType File -Force -Path $listPath | Out-Null

$sourceFiles = Get-ChildItem -Filter *.avi | Where-Object { $_.Name -notmatch "track1|track2|final_stitched|stacked" }
$existingTrack1 = Get-ChildItem -Filter *_track1.avi

$totalSplitTasks = $sourceFiles.Count
$totalStackTasks = $totalSplitTasks + $existingTrack1.Count
$script:totalTasks = $totalSplitTasks + $totalStackTasks + 2

# ==========================================
# STAGE 1: SPLITTING
# ==========================================
$stageCompleted = 0
foreach ($file in $sourceFiles) {
    $baseName = $file.BaseName
    $track1 = "${baseName}_track1.avi"
    $track2 = "${baseName}_track2.avi"
    
    if (-not (Test-Path $track1) -or -not (Test-Path $track2)) {
        Update-Dashboard -stageName "1/4: Splitting 2-Channel Videos" -stageTotal $totalSplitTasks -stageCompleted $stageCompleted -currentFile $file.Name -status "Processing (Fresh)" -statusColor "Green"
        
        $args = @("-fflags", "+genpts", "-i", "`"$($file.FullName)`"", "-map", "0:v:0", "-map", "0:a:0", "-c:v", "copy", "-c:a", "copy", "`"$track1`"", "-map", "0:v:1", "-map", "0:a:0", "-c:v", "copy", "-c:a", "copy", "`"$track2`"", "-y")
        Invoke-SilentProcess -exe "ffmpeg" -argArray $args -failMessage "Failed to split file: $($file.Name)"
        
        if ((Test-Path $track1) -and (Test-Path $track2)) { Send-ToRecycleBin -FilePath $file.FullName }
    } else {
        Update-Dashboard -stageName "1/4: Splitting 2-Channel Videos" -stageTotal $totalSplitTasks -stageCompleted $stageCompleted -currentFile $file.Name -status "Skipped (Resumed)" -statusColor "DarkGray"
    }
    $stageCompleted++
    $script:globalCompletedTasks++
}

# ==========================================
# STAGE 2: STACKING
# ==========================================
$track1Files = Get-ChildItem | Where-Object { $_.Name -match "track1" } | Sort-Object Name
$stageCompleted = 0

foreach ($file in $track1Files) {
    $ext = $file.Extension
    $baseName = $file.BaseName -replace "_track1", "" -replace "track1", ""
    $track1 = $file.Name
    $track2 = "${baseName}_track2$ext"
    $stackedOut = "temp_stacked\${baseName}_stacked$ext"
    
    if (Test-Path $track2) {
        if (-not (Test-Path $stackedOut)) {
            Update-Dashboard -stageName "2/4: Stacking Tracks Vertically" -stageTotal $totalStackTasks -stageCompleted $stageCompleted -currentFile $baseName -status "Processing (Fresh)" -statusColor "Green"
            
            $args = @("-i", "`"$track1`"", "-i", "`"$track2`"", "-filter_complex", "[0:v]scale=1920:-2,format=yuv420p[v0];[1:v]scale=1920:-2,format=yuv420p[v1];[v0][v1]vstack=inputs=2[v]", "-map", "[v]", "-map", "0:a:0", "-c:v", "libx264", "-preset", "fast", "-crf", "23", "-c:a", "aac", "-b:a", "192k", "`"$stackedOut`"", "-y")
            Invoke-SilentProcess -exe "ffmpeg" -argArray $args -failMessage "Failed to stack: $baseName"
            
            if (Test-Path $stackedOut) {
                Send-ToRecycleBin -FilePath $track1
                Send-ToRecycleBin -FilePath $track2
            }
        } else {
            Update-Dashboard -stageName "2/4: Stacking Tracks Vertically" -stageTotal $totalStackTasks -stageCompleted $stageCompleted -currentFile $baseName -status "Skipped (Resumed)" -statusColor "DarkGray"
        }
        Add-Content -Path $listPath -Value "file 'temp_stacked/${baseName}_stacked$ext'"
    }
    $stageCompleted++
    $script:globalCompletedTasks++
}

# ==========================================
# STAGE 3: STITCHING
# ==========================================
if ((Get-Content $listPath -ErrorAction SilentlyContinue | Measure-Object).Count -gt 0) {
    if (-not (Test-Path .\final_stitched_sequence.avi)) {
        Update-Dashboard -stageName "3/4: Stitching Final Sequence" -stageTotal 1 -stageCompleted 0 -currentFile "final_stitched_sequence.avi" -status "Processing (Fresh)" -statusColor "Green"
        
        $args = @("-fflags", "+genpts", "-f", "concat", "-safe", "0", "-i", "`"$listPath`"", "-c", "copy", "`"final_stitched_sequence.avi`"", "-y")
        Invoke-SilentProcess -exe "ffmpeg" -argArray $args -failMessage "Failed to stitch final sequence."

        if (Test-Path .\final_stitched_sequence.avi) {
            if (Test-Path .\temp_stacked) { Remove-Item -Recurse -Force .\temp_stacked }
            if (Test-Path $listPath) { Remove-Item -Force $listPath }
        }
    } else {
        Update-Dashboard -stageName "3/4: Stitching Final Sequence" -stageTotal 1 -stageCompleted 0 -currentFile "final_stitched_sequence.avi" -status "Skipped (Resumed)" -statusColor "DarkGray"
    }
}
$script:globalCompletedTasks++

# ==========================================
# STAGE 4: COMPRESSION
# ==========================================
$inputVideo = ".\final_stitched_sequence.avi"
$compressedVideo = ".\final_stitched_sequence_compressed.mp4"

$hbExe = "HandBrakeCLI"
if (Test-Path ".\HandBrakeCLI.exe") { $hbExe = ".\HandBrakeCLI.exe" }

if (Get-Command $hbExe -ErrorAction SilentlyContinue -or (Test-Path $hbExe)) {
    if (-not (Test-Path $compressedVideo)) {
        Update-Dashboard -stageName "4/4: Compressing (H.265)" -stageTotal 1 -stageCompleted 0 -currentFile "final_stitched_sequence_compressed.mp4" -status "Processing (Fresh)" -statusColor "Green"
        
        if (Test-Path $inputVideo) {
            $args = @("-i", "`"$inputVideo`"", "-o", "`"$compressedVideo`"", "-e", "x265", "-q", "22", "--encoder-preset", "fast", "--crop", "0:0:0:0", "-E", "copy")
            Invoke-SilentProcess -exe $hbExe -argArray $args -failMessage "HandBrake compression failed."
            
            if (Test-Path $compressedVideo) { Send-ToRecycleBin -FilePath $inputVideo }
        }
    } else {
        Update-Dashboard -stageName "4/4: Compressing (H.265)" -stageTotal 1 -stageCompleted 0 -currentFile "final_stitched_sequence_compressed.mp4" -status "Skipped (Resumed)" -statusColor "DarkGray"
    }
} else {
    Update-Dashboard -stageName "4/4: SKIPPED COMPRESSION (HandBrake Not Found)" -stageTotal 1 -stageCompleted 0 -currentFile "None" -status "Skipped (Missing Exe)" -statusColor "Yellow"
    Start-Sleep -Seconds 2
}
$script:globalCompletedTasks++


# ==========================================
# COMPLETE
# ==========================================
Update-Dashboard -stageName "COMPLETE" -stageTotal 1 -stageCompleted 1 -currentFile "All operations finished successfully." -status "Done" -statusColor "Green"
Write-Host "`n Script finished. Press any key to exit..." -ForegroundColor Green
$null = $Host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")


