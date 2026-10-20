// The oracle for packages/javamessageformat. Each input line is
// `<hex pattern> <arg>...` with `l:<long>`, `d:<hex bits>`, `s:<hex>` or `n`;
// each answer is the hex of MessageFormat's result for Locale.US, `bad` when
// the constructor refuses the pattern, `fmt` when format refuses an
// argument, or `index` for a choice with no intervals.
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.text.*;
import java.util.*;

public class MessageOracle {
    static String unhex(String h) {
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }
    static Object arg(String a) {
        String v = a.length() > 2 ? a.substring(2) : "";
        switch (a.charAt(0)) {
            case 'l': return Long.parseLong(v);
            case 'd': return Double.longBitsToDouble(Long.parseUnsignedLong(v, 16));
            case 's': return unhex(v);
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
            MessageFormat mf = null;
            try {
                mf = new MessageFormat(unhex(f[0]), Locale.US);
            } catch (IllegalArgumentException e) {
                r = "bad";
                out.append(r).append('\n');
                continue;
            }
            try {
                byte[] b = mf.format(args).getBytes(StandardCharsets.UTF_8);
                StringBuilder h = new StringBuilder();
                for (byte y : b) h.append(String.format("%02x", y));
                r = h.length() == 0 ? "-" : h.toString();
            } catch (IllegalArgumentException e) {
                r = "fmt";
            } catch (ArrayIndexOutOfBoundsException e) {
                r = "index";
            }
            out.append(r).append('\n');
        }
        System.out.print(out);
    }
}
