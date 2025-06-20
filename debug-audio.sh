#!/bin/bash

echo "🔍 AUDIO SYSTEM DIAGNOSTICS"
echo "=========================="
echo ""

echo "1. Checking if PulseAudio is running:"
if pgrep -x "pulseaudio" > /dev/null; then
    echo "   ✅ PulseAudio is running"
else
    echo "   ❌ PulseAudio is NOT running"
fi
echo ""

echo "2. Checking audio devices:"
if command -v pactl &> /dev/null; then
    echo "   Available sources (microphones):"
    pactl list sources short 2>/dev/null || echo "   ❌ pactl command failed"
else
    echo "   ❌ pactl command not found"
fi
echo ""

echo "3. Checking ALSA devices:"
if command -v arecord &> /dev/null; then
    echo "   ALSA recording devices:"
    arecord -l 2>/dev/null || echo "   ❌ No ALSA recording devices found"
else
    echo "   ❌ arecord command not found"
fi
echo ""

echo "4. Checking user permissions:"
groups | grep -q audio && echo "   ✅ User is in audio group" || echo "   ❌ User NOT in audio group"
echo ""

echo "5. Testing simple audio capture:"
if command -v arecord &> /dev/null; then
    echo "   Attempting 1-second test recording..."
    timeout 1 arecord -f cd -t raw /dev/null 2>/dev/null && echo "   ✅ Basic audio capture works" || echo "   ❌ Audio capture failed"
else
    echo "   ❌ Cannot test - arecord not available"
fi
echo ""

echo "6. Checking libpulse libraries:"
ldconfig -p | grep -q libpulse && echo "   ✅ PulseAudio libraries found" || echo "   ❌ PulseAudio libraries missing"
echo ""

echo "Run this script on your Pi to diagnose the audio issue!" 