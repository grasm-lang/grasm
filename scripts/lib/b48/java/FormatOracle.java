// The oracle for packages/javaformat: String.format(Locale.US, ...). Each
// input line is `<hex format> <arg>...` with arguments `i:<int>`, `l:<long>`,
// `b:<byte>`, `s:<short>`, `d:<hex bits>`, `f:<hex bits>`, `S:<hex utf-8>`,
// `c:<char code>`, `z:t|f` or `n`; each answer is the hex of the result, or
// `error` when Java throws.
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;

public class FormatOracle {
    static String unhex(String h) {
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }
    static Object arg(String a) {
        String v = a.length() > 2 ? a.substring(2) : "";
        switch (a.charAt(0)) {
            case 'i': return Integer.parseInt(v);
            case 'l': return Long.parseLong(v);
            case 'b': return Byte.parseByte(v);
            case 's': return Short.parseShort(v);
            case 'd': return Double.longBitsToDouble(Long.parseUnsignedLong(v, 16));
            case 'f': return Float.intBitsToFloat((int) Long.parseLong(v, 16));
            case 'S': return unhex(v);
            case 'c': return (char) Integer.parseInt(v);
            case 'z': return v.equals("t");
            default: return null;
        }
    }
    public static void main(String[] x) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in));
        StringBuilder out = new StringBuilder();
        String line;
        while ((line = in.readLine()) != null) {
            if (line.isEmpty()) continue;
            String[] f = line.split(" ");
            Object[] args = new Object[f.length - 1];
            for (int k = 1; k < f.length; k++) args[k - 1] = arg(f[k]);
            String r;
            try {
                byte[] b = String.format(Locale.US, unhex(f[0]), args).getBytes(StandardCharsets.UTF_8);
                StringBuilder h = new StringBuilder();
                for (byte y : b) h.append(String.format("%02x", y));
                r = h.length() == 0 ? "-" : h.toString();
            } catch (IllegalFormatException e) {
                r = "error";
            }
            out.append(r).append('\n');
        }
        System.out.print(out);
    }
}
