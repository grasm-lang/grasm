// Oracle for javasimpledateformat: Java 17's java.text.SimpleDateFormat
// (pattern, Locale.US) with time zone GMT. Input: <hex pattern> <millis>.
// Output: OK <hex text> | E <exception class> <hex message>.
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;
import java.util.TimeZone;

public class B58SdfOracle {
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
        TimeZone gmt = TimeZone.getTimeZone("GMT");
        String line;
        while ((line = in.readLine()) != null) {
            String[] f = line.split(" ");
            try {
                SimpleDateFormat fmt = new SimpleDateFormat(unhex(f[0]), Locale.US);
                fmt.setTimeZone(gmt);
                out.println("OK " + hex(fmt.format(new Date(Long.parseLong(f[1])))));
            } catch (Exception e) {
                out.println("E " + e.getClass().getName() + " " + hex(String.valueOf(e.getMessage())));
            }
        }
        out.flush();
    }
}
