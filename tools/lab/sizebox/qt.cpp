// Qt 6 size box: 200 x 200 device-independent pixels, magenta on white.
// resize, not setFixedSize: a fixed-size window kept its old size when moved
// to a monitor with another scale (found 2026-11-06), cutting the box off.
#include <QApplication>
#include <QPainter>
#include <QWidget>
struct Box : QWidget {
    void paintEvent(QPaintEvent *) override {
        QPainter p(this);
        p.fillRect(rect(), Qt::white);
        p.fillRect(20, 20, 200, 200, QColor(255, 0, 255));
    }
};
int main(int argc, char **argv) {
    QApplication app(argc, argv);
    Box b;
    b.setWindowTitle("vergence-box-qt");
    b.resize(240, 240);
    b.show();
    return app.exec();
}
