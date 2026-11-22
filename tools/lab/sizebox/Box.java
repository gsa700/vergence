// Java Swing size box: 200 x 200 in user space, magenta on white. Run: java Box.java
import javax.swing.*;
import java.awt.*;
public class Box {
    public static void main(String[] a) {
        SwingUtilities.invokeLater(() -> {
            JFrame f = new JFrame("vergence-box-java");
            JPanel p = new JPanel() {
                protected void paintComponent(Graphics g) {
                    g.setColor(Color.WHITE); g.fillRect(0, 0, getWidth(), getHeight());
                    g.setColor(new Color(255, 0, 255)); g.fillRect(20, 20, 200, 200);
                }
            };
            p.setPreferredSize(new Dimension(240, 240));
            f.add(p); f.pack(); f.setDefaultCloseOperation(JFrame.EXIT_ON_CLOSE); f.setVisible(true);
        });
    }
}
