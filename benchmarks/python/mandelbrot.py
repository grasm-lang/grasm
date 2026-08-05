def escapes(cx, cy):
    zx = 0.0
    zy = 0.0
    for i in range(200):
        if zx * zx + zy * zy > 4.0:
            return i
        tx = zx * zx - zy * zy + cx
        zy = 2.0 * zx * zy + cy
        zx = tx
    return 200


total = 0
for row in range(400):
    cy = row / 400.0 * 2.4 - 1.2
    for col in range(600):
        cx = col / 600.0 * 3.0 - 2.0
        if escapes(cx, cy) == 200:
            total += 1
print(total)
