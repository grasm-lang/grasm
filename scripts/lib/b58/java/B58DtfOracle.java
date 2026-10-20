// Oracle for javadatetimeformatter: Java 17's DateTimeFormatter.ofPattern
// (pattern, Locale.US) formatting a LocalDate (D), LocalTime (T) or
// LocalDateTime (DT). Input: <hex pattern> <kind> y m d h mi s nano.
// Output: OK <hex text> | E <exception class> <hex message>.
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.time.*;
import java.time.format.DateTimeFormatter;
import java.time.temporal.TemporalAccessor;
import java.util.Locale;

public class B58DtfOracle {
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
            String pattern = unhex(f[0]);
            int y = Integer.parseInt(f[2]), m = Integer.parseInt(f[3]), d = Integer.parseInt(f[4]);
            int h = Integer.parseInt(f[5]), mi = Integer.parseInt(f[6]), s = Integer.parseInt(f[7]), n = Integer.parseInt(f[8]);
            try {
                DateTimeFormatter fmt = DateTimeFormatter.ofPattern(pattern, Locale.US);
                TemporalAccessor t = f[1].equals("D") ? LocalDate.of(y, m, d) : f[1].equals("T") ? LocalTime.of(h, mi, s, n) : LocalDateTime.of(y, m, d, h, mi, s, n);
                out.println("OK " + hex(fmt.format(t)));
            } catch (Exception e) {
                out.println("E " + e.getClass().getName() + " " + hex(String.valueOf(e.getMessage())));
            }
        }
        out.flush();
    }
}
