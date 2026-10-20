// The oracle for packages/javaintegers: Java 17's Integer and Long text
// conversions. Text travels as hex of UTF-8 ("-" when empty); results are
// printed as signed decimals (an unsigned long as its bits), errors as
// E: and the message.
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;

public class IntegersOracle {
    static String unhex(String h) {
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }

    static String line(String[] f) {
        String op = f[0];
        switch (op) {
            case "pi": return Integer.toString(Integer.parseInt(unhex(f[1]), Integer.parseInt(f[2])));
            case "pl": return Long.toString(Long.parseLong(unhex(f[1]), Integer.parseInt(f[2])));
            case "pui": return Integer.toString(Integer.parseUnsignedInt(unhex(f[1]), Integer.parseInt(f[2])));
            case "pul": return Long.toString(Long.parseUnsignedLong(unhex(f[1]), Integer.parseInt(f[2])));
            case "di": return Integer.toString(Integer.decode(unhex(f[1])));
            case "dl": return Long.toString(Long.decode(unhex(f[1])));
            default: break;
        }
        long v = Long.parseLong(f[1]);
        int radix = Integer.parseInt(f[2]);
        boolean is32 = f[3].equals("32");
        switch (op) {
            case "ts": return is32 ? Integer.toString((int) v, radix) : Long.toString(v, radix);
            case "tus": return is32 ? Integer.toUnsignedString((int) v, radix) : Long.toUnsignedString(v, radix);
            case "th": return is32 ? Integer.toHexString((int) v) : Long.toHexString(v);
            case "to": return is32 ? Integer.toOctalString((int) v) : Long.toOctalString(v);
            default: return is32 ? Integer.toBinaryString((int) v) : Long.toBinaryString(v);
        }
    }

    public static void main(String[] args) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        StringBuilder out = new StringBuilder();
        String l;
        while ((l = in.readLine()) != null) {
            if (l.isEmpty()) continue;
            String[] f = l.split(" ");
            String[] g = new String[f.length - 1];
            System.arraycopy(f, 1, g, 0, g.length);
            String r;
            try {
                r = line(g);
            } catch (NumberFormatException e) {
                r = "E:" + e.getMessage();
            }
            out.append(r).append('\n');
        }
        // A lone surrogate goes out as U+FFFD, as the package writes it.
        String text = out.toString().replaceAll("[\\uD800-\\uDBFF](?![\\uDC00-\\uDFFF])|(?<![\\uD800-\\uDBFF])[\\uDC00-\\uDFFF]", "\uFFFD");
        new java.io.PrintStream(System.out, true, "UTF-8").print(text);
    }
}
