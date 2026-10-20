// The oracle for packages/javalocaldate: Java 17's java.time.LocalDate.
// Dates travel as y,m,d; text as hex of UTF-8, '-' when empty.
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.time.LocalDate;
import java.time.temporal.ChronoUnit;

public class LocalDateOracle {
    static String hx(String s) {
        if (s.isEmpty()) return "-";
        StringBuilder sb = new StringBuilder();
        // An unpaired surrogate becomes U+FFFD, as the package writes it.
        for (byte b : s.replaceAll("[\\uD800-\\uDBFF](?![\\uDC00-\\uDFFF])|(?<![\\uD800-\\uDBFF])[\\uDC00-\\uDFFF]", "�").getBytes(StandardCharsets.UTF_8))
            sb.append(String.format("%02x", b & 0xff));
        return sb.toString();
    }

    static String unhex(String h) {
        if (h.equals("-")) return "";
        byte[] b = new byte[h.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(h.substring(2 * i, 2 * i + 2), 16);
        return new String(b, StandardCharsets.UTF_8);
    }

    static LocalDate date(String s) {
        String[] p = s.split(",");
        return LocalDate.of(Integer.parseInt(p[0]), Integer.parseInt(p[1]), Integer.parseInt(p[2]));
    }

    static String show(LocalDate d) {
        return d.getYear() + "," + d.getMonthValue() + "," + d.getDayOfMonth() + "|" + hx(d.toString());
    }

    static String line(String[] f) {
        switch (f[0]) {
            case "parse": return show(LocalDate.parse(unhex(f[1])));
            case "of": return show(LocalDate.of(Integer.parseInt(f[1]), Integer.parseInt(f[2]), Integer.parseInt(f[3])));
            case "epoch": return show(LocalDate.ofEpochDay(Long.parseLong(f[1])));
            case "info": {
                LocalDate d = date(f[1]);
                return d.toEpochDay() + "|" + d.getDayOfWeek() + "|" + d.getDayOfYear() + "|" + d.lengthOfMonth() + "|" + d.lengthOfYear() + "|" + d.isLeapYear();
            }
            case "plus": {
                LocalDate d = date(f[1]);
                long n = Long.parseLong(f[3]);
                switch (f[2]) {
                    case "d": return show(d.plusDays(n));
                    case "w": return show(d.plusWeeks(n));
                    case "m": return show(d.plusMonths(n));
                    default: return show(d.plusYears(n));
                }
            }
            case "with": {
                LocalDate d = date(f[1]);
                int n = Integer.parseInt(f[3]);
                switch (f[2]) {
                    case "y": return show(d.withYear(n));
                    case "m": return show(d.withMonth(n));
                    default: return show(d.withDayOfMonth(n));
                }
            }
            default: return Long.toString(date(f[1]).until(date(f[2]), ChronoUnit.valueOf(f[3])));
        }
    }

    public static void main(String[] args) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        StringBuilder out = new StringBuilder();
        String l;
        while ((l = in.readLine()) != null) {
            if (l.isEmpty()) continue;
            String r;
            try {
                r = line(l.split(" "));
            } catch (RuntimeException e) {
                r = "E:" + e.getClass().getSimpleName() + ":" + hx(e.getMessage());
            }
            out.append(r).append('\n');
        }
        System.out.print(out);
    }
}
