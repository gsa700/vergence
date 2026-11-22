// Electron size box: 200 x 200 CSS pixels, magenta on white. The window is
// large because Electron 42 on X11 scales its content but not its window
// (found 2026-11-06): at scale 2 a 240 px window cut the box off.
const { app, BrowserWindow } = require('electron');
app.whenReady().then(() => {
  const w = new BrowserWindow({ width: 600, height: 600, useContentSize: true, title: 'vergence-box-electron', autoHideMenuBar: true });
  w.loadURL('data:text/html,<title>vergence-box-electron</title><body style="margin:0;background:white"><div style="position:absolute;left:20px;top:20px;width:200px;height:200px;background:%23ff00ff"></div>');
});
