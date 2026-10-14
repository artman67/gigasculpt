GigaSculpt
==========

Version @VERSION@.

GigaSculpt is a free, open-source digital sculpting app for pen and mouse.
It is licensed under the GNU GPL, version 3 or later: see the LICENSE file
beside this one.


What is in this download
------------------------

gigasculpt, the app, and gs, its command-line tool for scripts and AI
agents, with Quad Remesh's helper program in a folder named quadwild. Keep
them together: gs finds the app beside itself, and the app finds the
helper there. On macOS all of them are inside GigaSculpt.app.


Starting it
-----------

Linux:    run ./gigasculpt in this folder. It needs a graphics driver with
          Vulkan (Mesa or your card maker's).

Windows:  run gigasculpt.exe. In a terminal opened in this folder, typing
          gigasculpt (.\gigasculpt in PowerShell) runs it there, so
          gigasculpt --help and gigasculpt --version print in the terminal.

macOS:    drag GigaSculpt to Applications and open it from there.

These builds are not signed yet, so Windows and macOS warn the first time.
On Windows, click "More info", then "Run anyway". On macOS, open it once,
then open System Settings > Privacy & Security, click "Open Anyway" beside
GigaSculpt and confirm.


The guide
---------

@GUIDE@

It shows how to sculpt, page by page, and its Troubleshooting page says
what to do when something goes wrong. The same pages are in the docs
folder of GigaSculpt's source.
