@echo off
rem Runs a spike windowed, with the traffic debug overlay off.
rem Pass the spike filename as the first argument, defaults to spike_left_turn.gd.
set METRUM_DEBUG_TRAFFIC=
set SPIKE=%1
if "%SPIKE%"=="" set SPIKE=spike_left_turn.gd
cd /d C:\Users\David\Documents\metrum_rise
"C:\Users\David\Downloads\Godot_v4.7.1-stable_mono_win64\Godot_v4.7.1-stable_mono_win64\Godot_v4.7.1-stable_mono_win64_console.exe" --path godot --script %SPIKE% > spike.log 2>&1
