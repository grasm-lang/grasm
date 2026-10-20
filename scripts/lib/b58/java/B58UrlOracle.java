// Oracle for javaurlencoder: Java 17's URLEncoder.encode and
// URLDecoder.decode with UTF-8. Input lines: e <hex> | d <hex>; text and
// messages cross as hex of UTF-8 (an unpaired surrogate written as '?').
import java.io.*;
import java.net.URLDecoder;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;

public class B58UrlOracle {
    static String unhex(String h) {
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }

    static String hex(String s) {
        byte[] b = s.getBytes(StandardCharsets.UTF_8);
        if (b.length == 0) return "-";
        StringBuilder sb = new StringBuilder();
        for (byte x : b) sb.append(String.format("%02x", x & 0xff));
        return sb.toString();
    }

    public static void main(String[] args) throws IOException {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        PrintWriter out = new PrintWriter(new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8)));
        String line;
        while ((line = in.readLine()) != null) {
            String[] f = line.split(" ");
            String s = unhex(f[1]);
            if (f[0].equals("e")) {
                out.println(hex(URLEncoder.encode(s, StandardCharsets.UTF_8)));
            } else {
                try {
                    out.println("OK " + hex(URLDecoder.decode(s, StandardCharsets.UTF_8)));
                } catch (IllegalArgumentException e) {
                    out.println("E " + hex(e.getMessage()));
                }
            }
        }
        out.flush();
    }
}
