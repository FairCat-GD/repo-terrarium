Repo Terrarium: the Windows pinning probe
==========================================

Thank you for testing. Repo Terrarium is a thin pixel-art strip that sits along
the bottom edge of a monitor. On Windows it reserves that edge, the way the
taskbar does, so maximised windows stop above it. Nobody on this project has a
Windows machine, so this probe is how we find out whether that works.

It takes about ten minutes. It changes nothing on your machine: there is no
installer, and it writes only its report files, into this folder.


Before you start
----------------

1. Extract the zip first (right-click it, then Extract All). Running it from
   inside the zip does not work.
2. Windows may say "Windows protected your PC", because the app is not signed.
   Choose "More info", then "Run anyway".
3. In this build the strip stays on top of full-screen video on its display.
   That is expected; the probe measures it.


Run 1: the walk-through
-----------------------

Double-click run-probe.cmd. A console window opens and the strip appears.

The console describes one thing to try at a time. Try it, then come back to the
console: type a short note if something is worth saying, and press Enter to go
on. Enter alone is fine too.

The last step asks you to end the probe from a second terminal with:

    taskkill /f /im repo-terrarium.exe

That ends it the way a crash would, on purpose. Then maximise a window on the
same display and look whether it stops short of the bottom, as if the strip
were still there.


Run 2 and run 3: two short checks
---------------------------------

Run 2. Double-click run-probe.cmd again. At the first step, type a note saying
whether the maximised window stopped short of the bottom before this run, then
look at the strip and press Enter. Now press Ctrl-C in the console. The probe
ends at once.

Run 3. Double-click run-probe.cmd once more. At the first step, type a note
saying whether a maximised window still stopped short of the bottom after run 2,
then press Enter. End this run with Ctrl-Z and then Enter.


Sending the results
-------------------

Each run writes two files into this folder, named
repo-terrarium-probe-pinning-<numbers>.json and .txt. Please send all of the
repo-terrarium-probe- files back. They hold the display sizes, what Windows
reported about the strip, the names of window classes (never window titles),
and your notes. Nothing else.

If the strip ever gets stuck on screen, or the bottom of the screen stays
reserved after the probe has ended, signing out and back in clears it.
