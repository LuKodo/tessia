// Registers every page (`page-*.ts`); a new page just adds its own file, there is no list to edit.
import.meta.glob("./page-*.ts", { eager: true });
