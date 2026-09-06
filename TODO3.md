Mouse wheel scrolling seems to go the wrong way. Make sure we're following the direction properly
since the system settings can change the direction for the wheel.

I tried loading and saving commands like ":e foo.txt" or ":s bar.txt" and they didn't work. The
":e" command should replace the current window (assuming it's been saved). Let's define ":E" to open
in a new window.

I tried ":pwd" and it didn't work.
