#include <stdio.h>
static int escapes(double cx, double cy) {
    double zx = 0.0, zy = 0.0;
    for (int i = 0; i < 200; i++) {
        if (zx * zx + zy * zy > 4.0) return i;
        double tx = zx * zx - zy * zy + cx;
        zy = 2.0 * zx * zy + cy;
        zx = tx;
    }
    return 200;
}
int main(void) {
    long total = 0;
    for (int row = 0; row < 400; row++) {
        double cy = row / 400.0 * 2.4 - 1.2;
        for (int col = 0; col < 600; col++) {
            double cx = col / 600.0 * 3.0 - 2.0;
            if (escapes(cx, cy) == 200) total++;
        }
    }
    printf("%ld\n", total);
    return 0;
}
