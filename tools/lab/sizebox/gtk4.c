/* GTK 4 size box: 200 x 200 logical pixels, magenta on white. */
#include <gtk/gtk.h>
static void draw(GtkDrawingArea *a, cairo_t *cr, int w, int h, gpointer d) {
    cairo_set_source_rgb(cr, 1, 1, 1); cairo_paint(cr);
    cairo_set_source_rgb(cr, 1, 0, 1); cairo_rectangle(cr, 20, 20, 200, 200); cairo_fill(cr);
}
static void activate(GtkApplication *app, gpointer d) {
    GtkWidget *win = gtk_application_window_new(app);
    gtk_window_set_title(GTK_WINDOW(win), "vergence-box-gtk4");
    GtkWidget *a = gtk_drawing_area_new();
    gtk_drawing_area_set_content_width(GTK_DRAWING_AREA(a), 240);
    gtk_drawing_area_set_content_height(GTK_DRAWING_AREA(a), 240);
    gtk_drawing_area_set_draw_func(GTK_DRAWING_AREA(a), draw, NULL, NULL);
    gtk_window_set_child(GTK_WINDOW(win), a);
    gtk_window_present(GTK_WINDOW(win));
}
int main(int argc, char **argv) {
    GtkApplication *app = gtk_application_new("org.vergence.box.gtk4", G_APPLICATION_NON_UNIQUE);
    g_signal_connect(app, "activate", G_CALLBACK(activate), NULL);
    return g_application_run(G_APPLICATION(app), argc, argv);
}
