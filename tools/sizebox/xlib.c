/* Xlib size box: 200 x 200 device pixels (Xlib has no scaling). */
#include <X11/Xlib.h>
#include <X11/Xutil.h>
int main(void) {
    Display *d = XOpenDisplay(NULL);
    if (!d) return 1;
    int s = DefaultScreen(d);
    Window w = XCreateSimpleWindow(d, RootWindow(d, s), 0, 0, 240, 240, 0, 0, WhitePixel(d, s));
    XStoreName(d, w, "vergence-box-xlib");
    XSelectInput(d, w, ExposureMask);
    XMapWindow(d, w);
    GC gc = XCreateGC(d, w, 0, NULL);
    XSetForeground(d, gc, 0xFF00FF);
    for (;;) {
        XEvent e;
        XNextEvent(d, &e);
        if (e.type == Expose) XFillRectangle(d, w, gc, 20, 20, 200, 200);
    }
}
