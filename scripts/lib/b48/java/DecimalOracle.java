// The oracle for packages/javadecimalformat. Each input line is
// `<hex pattern> <rounding mode> d:<hex bits>|l:<long>`; each answer is the
// hex of what DecimalFormat wrote, `bad` for a pattern Java refuses, or
// `arith` when rounding was needed under UNNECESSARY.
import java.io.*;
import java.math.RoundingMode;
import java.nio.charset.StandardCharsets;
import java.text.*;
import java.util.*;

public class DecimalOracle {
    static String unhex(String h) {
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }
    public static void main(String[] x) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in));
        DecimalFormatSymbols us = DecimalFormatSymbols.getInstance(Locale.US);
        StringBuilder out = new StringBuilder();
        String line;
        while ((line = in.readLine()) != null) {
            if (line.isEmpty()) continue;
            String[] f = line.split(" ");
            String r;
            try {
                DecimalFormat df = new DecimalFormat(unhex(f[0]), us);
                df.setRoundingMode(RoundingMode.valueOf(f[1]));
                String v = f[2].substring(2);
                String s = f[2].charAt(0) == 'd' ? df.format(Double.longBitsToDouble(Long.parseUnsignedLong(v, 16))) : df.format(Long.parseLong(v));
                byte[] b = s.getBytes(StandardCharsets.UTF_8);
                StringBuilder h = new StringBuilder();
                for (byte y : b) h.append(String.format("%02x", y));
                r = h.length() == 0 ? "-" : h.toString();
            } catch (IllegalArgumentException e) {
                r = "bad";
            } catch (ArithmeticException e) {
                r = "arith";
            }
            out.append(r).append('\n');
        }
        System.out.print(out);
    }
}
